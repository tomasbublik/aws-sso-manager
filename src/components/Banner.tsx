interface BannerProps {
  message: string;
  kind?: "error" | "warning";
  onDismiss?: () => void;
}

export function Banner({ message, kind = "error", onDismiss }: BannerProps) {
  return (
    <div className={`banner banner-${kind}`} role="alert">
      <span className="banner-message">{message}</span>
      {onDismiss && (
        <button
          type="button"
          className="banner-dismiss"
          aria-label="Dismiss"
          onClick={onDismiss}
        >
          ×
        </button>
      )}
    </div>
  );
}
