//! Line-preserving reader/writer for `~/.aws/config` (SPEC.md §5).
//!
//! The core edit functions operate purely on `String` (text in, text out) so
//! they can be unit tested without touching the real home directory. Only
//! [`read_config_text`] and [`write_config_text`] perform any file I/O; every
//! other public function is a pure text transformation.
//!
//! Editing model: the file is split into an ordered list of "sections" —
//! either the free-form preamble before the first `[...]` header, or the
//! block of text starting at a header line and running up to (but not
//! including) the next header line. Every byte of a section we do not touch
//! is preserved exactly, including comments, blank lines and key order.
//! Sections are only ever replaced in place or removed outright; new
//! sections are appended at the end. This means the app never reorders or
//! reformats content it does not own.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{AppError, AppResult};

/// An `[sso-session <name>]` section as read from the config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsoSessionInfo {
    pub name: String,
    pub start_url: String,
    pub sso_region: String,
}

impl From<&SsoSessionInfo> for crate::sso::SsoSession {
    fn from(info: &SsoSessionInfo) -> Self {
        crate::sso::SsoSession {
            name: info.name.clone(),
            start_url: info.start_url.clone(),
            sso_region: info.sso_region.clone(),
        }
    }
}

/// A profile managed by this app (has `sso_session = <our session>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedProfile {
    pub profile_name: String,
    pub account_id: String,
    pub role_name: String,
    pub region: String,
    pub is_default: bool,
}

/// The fields needed to render a `[profile X]` or `[default]` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileWrite {
    pub profile_name: String,
    pub account_id: String,
    pub role_name: String,
    pub region: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SectionKind {
    /// Free-form text before the first section header (comments, blank lines).
    Preamble,
    Profile(String),
    SsoSession(String),
    Default,
    /// Any other section type the app does not understand — never touched.
    Other(String),
}

#[derive(Debug, Clone)]
struct Section {
    kind: SectionKind,
    /// The exact original (or freshly rendered) bytes of this section,
    /// including its header line.
    raw: String,
}

fn classify_header(trimmed: &str) -> Option<SectionKind> {
    if trimmed.len() < 2 || !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = trimmed[1..trimmed.len() - 1].trim();
    if inner == "default" {
        Some(SectionKind::Default)
    } else if let Some(rest) = inner.strip_prefix("profile ") {
        Some(SectionKind::Profile(rest.trim().to_string()))
    } else if let Some(rest) = inner.strip_prefix("sso-session ") {
        Some(SectionKind::SsoSession(rest.trim().to_string()))
    } else {
        Some(SectionKind::Other(inner.to_string()))
    }
}

fn parse_sections(text: &str) -> Vec<Section> {
    let mut sections = Vec::new();
    let mut current_kind = SectionKind::Preamble;
    let mut current_raw = String::new();

    for line in text.split_inclusive('\n') {
        if let Some(kind) = classify_header(line.trim()) {
            if !(current_kind == SectionKind::Preamble && current_raw.is_empty()) {
                sections.push(Section {
                    kind: current_kind,
                    raw: current_raw,
                });
            }
            current_kind = kind;
            current_raw = String::new();
        }
        current_raw.push_str(line);
    }
    if !(current_kind == SectionKind::Preamble && current_raw.is_empty()) {
        sections.push(Section {
            kind: current_kind,
            raw: current_raw,
        });
    }
    sections
}

fn render(sections: &[Section]) -> String {
    sections.iter().map(|s| s.raw.as_str()).collect()
}

/// Reads a `key = value` line out of a section's raw text (skipping its
/// header line, comments and blank lines).
fn get_key(raw: &str, key: &str) -> Option<String> {
    for line in raw.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with(';') || t.starts_with('[') {
            continue;
        }
        if let Some((k, v)) = t.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

fn is_managed(raw: &str, session_name: &str) -> bool {
    get_key(raw, "sso_session").as_deref() == Some(session_name)
}

// ---------------------------------------------------------------------
// Profile naming (SPEC.md §5): <accountName>-<roleName> with longest
// dash-segment overlap collapsed, case-insensitively.
// ---------------------------------------------------------------------

pub fn profile_name_for(account_name: &str, role_name: &str) -> String {
    let account_segs: Vec<&str> = account_name.split('-').collect();
    let role_segs: Vec<&str> = role_name.split('-').collect();
    let max_k = account_segs.len().min(role_segs.len());

    let mut overlap = 0;
    for k in (1..=max_k).rev() {
        let a_tail = &account_segs[account_segs.len() - k..];
        let r_head = &role_segs[..k];
        if a_tail
            .iter()
            .zip(r_head.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
        {
            overlap = k;
            break;
        }
    }

    let mut segs: Vec<&str> = account_segs;
    segs.extend_from_slice(&role_segs[overlap..]);
    segs.join("-")
}

// ---------------------------------------------------------------------
// Read-only queries
// ---------------------------------------------------------------------

/// Returns the first `[sso-session ...]` section found in the file, if any.
pub fn first_sso_session(text: &str) -> Option<SsoSessionInfo> {
    for s in parse_sections(text) {
        if let SectionKind::SsoSession(name) = &s.kind {
            return Some(SsoSessionInfo {
                name: name.clone(),
                start_url: get_key(&s.raw, "sso_start_url").unwrap_or_default(),
                sso_region: get_key(&s.raw, "sso_region").unwrap_or_default(),
            });
        }
    }
    None
}

/// Returns every `[profile X]` section managed by this app for `session_name`
/// (i.e. it has `sso_session = <session_name>`). `is_default` is set when a
/// managed `[default]` section exists with the same account id + role name.
pub fn managed_profiles(text: &str, session_name: &str) -> Vec<ManagedProfile> {
    let sections = parse_sections(text);

    let default_fields = sections.iter().find_map(|s| {
        if s.kind == SectionKind::Default && is_managed(&s.raw, session_name) {
            Some((
                get_key(&s.raw, "sso_account_id").unwrap_or_default(),
                get_key(&s.raw, "sso_role_name").unwrap_or_default(),
            ))
        } else {
            None
        }
    });

    let mut out = Vec::new();
    for s in &sections {
        if let SectionKind::Profile(name) = &s.kind {
            if is_managed(&s.raw, session_name) {
                let account_id = get_key(&s.raw, "sso_account_id").unwrap_or_default();
                let role_name = get_key(&s.raw, "sso_role_name").unwrap_or_default();
                let region = get_key(&s.raw, "region").unwrap_or_default();
                let is_default = default_fields
                    .as_ref()
                    .map(|(a, r)| *a == account_id && *r == role_name)
                    .unwrap_or(false);
                out.push(ManagedProfile {
                    profile_name: name.clone(),
                    account_id,
                    role_name,
                    region,
                    is_default,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------------
// Section rendering
// ---------------------------------------------------------------------

fn render_profile_block(profile: &ProfileWrite, session_name: &str) -> String {
    format!(
        "[profile {name}]\nsso_session = {session}\nsso_account_id = {account_id}\nsso_role_name = {role}\nregion = {region}\noutput = json\n",
        name = profile.profile_name,
        session = session_name,
        account_id = profile.account_id,
        role = profile.role_name,
        region = profile.region,
    )
}

fn render_default_block(profile: &ProfileWrite, session_name: &str) -> String {
    format!(
        "[default]\nsso_session = {session}\nsso_account_id = {account_id}\nsso_role_name = {role}\nregion = {region}\noutput = json\n",
        session = session_name,
        account_id = profile.account_id,
        role = profile.role_name,
        region = profile.region,
    )
}

fn render_sso_session_block(session: &SsoSessionInfo) -> String {
    format!(
        "[sso-session {name}]\nsso_start_url = {url}\nsso_region = {region}\nsso_registration_scopes = sso:account:access\n",
        name = session.name,
        url = session.start_url,
        region = session.sso_region,
    )
}

fn append_block(text: &str, block: &str) -> String {
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out.push_str(block);
    out
}

// ---------------------------------------------------------------------
// Core edit functions (String in, String out)
// ---------------------------------------------------------------------

/// Inserts or replaces the `[sso-session <name>]` section.
pub fn upsert_sso_session(text: &str, session: &SsoSessionInfo) -> String {
    let mut sections = parse_sections(text);
    let block = render_sso_session_block(session);
    if let Some(s) = sections.iter_mut().find(|s| {
        matches!(&s.kind, SectionKind::SsoSession(n) if n == &session.name)
    }) {
        s.raw = block;
        render(&sections)
    } else {
        append_block(text, &block)
    }
}

/// Inserts or replaces a managed `[profile X]` section. Refuses to touch a
/// same-named section that isn't managed by us.
pub fn upsert_profile(text: &str, session_name: &str, profile: &ProfileWrite) -> Result<String, String> {
    let mut sections = parse_sections(text);
    let block = render_profile_block(profile, session_name);
    if let Some(idx) = sections
        .iter()
        .position(|s| matches!(&s.kind, SectionKind::Profile(n) if n == &profile.profile_name))
    {
        if is_managed(&sections[idx].raw, session_name) {
            sections[idx].raw = block;
            Ok(render(&sections))
        } else {
            Err(format!(
                "A profile named '{}' already exists in ~/.aws/config and is not managed by {}. Please rename or remove it manually.",
                profile.profile_name,
                crate::branding::APP_NAME
            ))
        }
    } else {
        Ok(append_block(text, &block))
    }
}

/// Removes a managed `[profile X]` section. No-op if it's absent or foreign.
pub fn remove_profile(text: &str, session_name: &str, profile_name: &str) -> String {
    let sections = parse_sections(text);
    let kept: Vec<Section> = sections
        .into_iter()
        .filter(|s| {
            !(matches!(&s.kind, SectionKind::Profile(n) if n == profile_name)
                && is_managed(&s.raw, session_name))
        })
        .collect();
    render(&kept)
}

/// Writes `[default]` with the given profile's fields. Only touches
/// `[default]` if it is absent or already managed by us — otherwise returns
/// an explanatory error (SPEC.md §5).
pub fn set_default(text: &str, session_name: &str, profile: &ProfileWrite) -> Result<String, String> {
    let mut sections = parse_sections(text);
    let block = render_default_block(profile, session_name);
    if let Some(idx) = sections.iter().position(|s| s.kind == SectionKind::Default) {
        if is_managed(&sections[idx].raw, session_name) {
            sections[idx].raw = block;
            Ok(render(&sections))
        } else {
            Err(format!(
                "A [default] profile already exists in ~/.aws/config and is not managed by {}. \
                 Remove or rename it manually before setting a default profile here.",
                crate::branding::APP_NAME
            ))
        }
    } else {
        Ok(append_block(text, &block))
    }
}

/// Removes `[default]` if it is managed by us. No-op if absent or foreign.
pub fn remove_default(text: &str, session_name: &str) -> String {
    let sections = parse_sections(text);
    let kept: Vec<Section> = sections
        .into_iter()
        .filter(|s| !(s.kind == SectionKind::Default && is_managed(&s.raw, session_name)))
        .collect();
    render(&kept)
}

// ---------------------------------------------------------------------
// Thin path wrappers — the only functions in this module that touch disk.
// ---------------------------------------------------------------------

fn home_dir() -> AppResult<PathBuf> {
    dirs::home_dir().ok_or_else(|| AppError::msg("Cannot determine home directory"))
}

fn aws_dir() -> AppResult<PathBuf> {
    Ok(home_dir()?.join(".aws"))
}

pub fn config_path() -> AppResult<PathBuf> {
    Ok(aws_dir()?.join("config"))
}

fn ensure_aws_dir() -> AppResult<PathBuf> {
    let dir = aws_dir()?;
    fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

/// Reads `~/.aws/config`, creating an empty file with restrictive
/// permissions if it (or the `~/.aws` directory) doesn't exist yet.
pub fn read_config_text() -> AppResult<String> {
    ensure_aws_dir()?;
    let path = config_path()?;
    if !path.exists() {
        fs::write(&path, "")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
        return Ok(String::new());
    }
    Ok(fs::read_to_string(path)?)
}

/// Set once the first modifying write of this process run has backed up the
/// existing config to `config.bak`.
static BACKED_UP_THIS_RUN: AtomicBool = AtomicBool::new(false);

/// Writes `~/.aws/config`, backing up the previous contents to `config.bak`
/// (overwriting any earlier backup) before the first modifying write of this
/// process run.
pub fn write_config_text(text: &str) -> AppResult<()> {
    ensure_aws_dir()?;
    let path = config_path()?;

    if BACKED_UP_THIS_RUN
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
        && path.exists()
    {
        let backup_path = path.with_extension("bak");
        let _ = fs::copy(&path, &backup_path);
    }

    fs::write(&path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_name_overlap_collapses_shared_segment() {
        assert_eq!(
            profile_name_for("acme-buildsys", "buildsys-users"),
            "acme-buildsys-users"
        );
    }

    #[test]
    fn profile_name_no_overlap_plain_join() {
        assert_eq!(profile_name_for("acme", "admin"), "acme-admin");
    }

    #[test]
    fn profile_name_overlap_is_case_insensitive() {
        assert_eq!(
            profile_name_for("Acme-BuildSys", "buildsys-Users"),
            "Acme-BuildSys-Users"
        );
    }

    #[test]
    fn profile_name_single_segment_no_overlap() {
        assert_eq!(profile_name_for("acme", "readonly"), "acme-readonly");
    }

    const FIXTURE: &str = "\
# personal profiles - do not touch
[profile personal]
region = us-east-1
output = json

[profile acme-old-role]
sso_session = acme
sso_account_id = 111111111111
sso_role_name = old-role
region = eu-central-1
output = json
";

    #[test]
    fn upsert_appends_new_profile_and_preserves_foreign_bytes() {
        let new_profile = ProfileWrite {
            profile_name: "acme-buildsys-users".into(),
            account_id: "111122223333".into(),
            role_name: "buildsys-users".into(),
            region: "eu-central-1".into(),
        };
        let after_add = upsert_profile(FIXTURE, "acme", &new_profile).unwrap();

        assert!(after_add.contains(
            "# personal profiles - do not touch\n[profile personal]\nregion = us-east-1\noutput = json\n"
        ));
        assert!(after_add.contains("[profile acme-buildsys-users]"));
        assert!(after_add.contains("sso_account_id = 111122223333"));
    }

    #[test]
    fn remove_deletes_only_the_managed_section() {
        let after_remove = remove_profile(FIXTURE, "acme", "acme-old-role");
        assert!(!after_remove.contains("acme-old-role"));
        assert!(after_remove.contains(
            "# personal profiles - do not touch\n[profile personal]\nregion = us-east-1\noutput = json\n"
        ));
    }

    #[test]
    fn remove_of_foreign_or_absent_profile_is_noop() {
        assert_eq!(remove_profile(FIXTURE, "acme", "personal"), FIXTURE);
        assert_eq!(remove_profile(FIXTURE, "acme", "does-not-exist"), FIXTURE);
    }

    #[test]
    fn upsert_refuses_to_overwrite_foreign_profile_with_same_name() {
        let clashing = ProfileWrite {
            profile_name: "personal".into(),
            account_id: "1".into(),
            role_name: "r".into(),
            region: "eu-central-1".into(),
        };
        let err = upsert_profile(FIXTURE, "acme", &clashing).unwrap_err();
        assert!(err.contains("personal"));
    }

    #[test]
    fn default_round_trip() {
        let profile = ProfileWrite {
            profile_name: "acme-buildsys-users".into(),
            account_id: "111122223333".into(),
            role_name: "buildsys-users".into(),
            region: "eu-central-1".into(),
        };
        let with_default = set_default(FIXTURE, "acme", &profile).unwrap();
        assert!(with_default.contains("[default]"));
        assert!(with_default.contains("sso_session = acme"));

        // A second call to a managed [default] section replaces it in place.
        let profile2 = ProfileWrite {
            region: "eu-west-1".into(),
            ..profile
        };
        let replaced = set_default(&with_default, "acme", &profile2).unwrap();
        assert!(replaced.contains("region = eu-west-1"));
        assert_eq!(replaced.matches("[default]").count(), 1);

        let removed = remove_default(&replaced, "acme");
        assert!(!removed.contains("[default]"));
        assert!(removed.contains(
            "# personal profiles - do not touch\n[profile personal]\nregion = us-east-1\noutput = json\n"
        ));
    }

    #[test]
    fn set_default_refuses_foreign_default() {
        let foreign = "[default]\nregion = us-east-1\noutput = json\n";
        let profile = ProfileWrite {
            profile_name: "acme-buildsys-users".into(),
            account_id: "111122223333".into(),
            role_name: "buildsys-users".into(),
            region: "eu-central-1".into(),
        };
        let err = set_default(foreign, "acme", &profile).unwrap_err();
        assert!(err.contains("[default]"));
    }

    #[test]
    fn sso_session_round_trip() {
        let session = SsoSessionInfo {
            name: "acme".into(),
            start_url: "https://acme.awsapps.com/start".into(),
            sso_region: "eu-central-1".into(),
        };
        let out = upsert_sso_session("", &session);
        let parsed = first_sso_session(&out).unwrap();
        assert_eq!(parsed, session);
    }

    #[test]
    fn managed_profiles_reports_default_flag() {
        let profile = ProfileWrite {
            profile_name: "acme-buildsys-users".into(),
            account_id: "111122223333".into(),
            role_name: "buildsys-users".into(),
            region: "eu-central-1".into(),
        };
        let text = upsert_profile(FIXTURE, "acme", &profile).unwrap();
        let text = set_default(&text, "acme", &profile).unwrap();

        let profiles = managed_profiles(&text, "acme");
        // "acme-old-role" and "acme-buildsys-users" are managed; "personal" is not.
        assert_eq!(profiles.len(), 2);
        let new_one = profiles
            .iter()
            .find(|p| p.profile_name == "acme-buildsys-users")
            .unwrap();
        assert!(new_one.is_default);
        let old_one = profiles
            .iter()
            .find(|p| p.profile_name == "acme-old-role")
            .unwrap();
        assert!(!old_one.is_default);
    }
}
