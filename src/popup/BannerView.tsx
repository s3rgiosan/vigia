import { Icon } from "../components/Icon";
import { authReason } from "../lib/auth";
import { bannerMessage, type Banner } from "../lib/grouping";
import type { SettingsTarget } from "../lib/tauri";

/** One persistent banner. Errors are announced as alerts, warnings as status messages. */
export function BannerView({
  banner,
  onOpenSettings,
}: {
  banner: Banner;
  onOpenSettings: (target: SettingsTarget) => void;
}) {
  const message = bannerMessage(banner);
  switch (banner.kind) {
    case "secrets_blocked":
      return (
        <div className="banner banner--error" role="alert">
          <Icon name="warning" size={14} />
          <span className="banner__text">{message}</span>
          <button type="button" className="banner__action" onClick={() => onOpenSettings({ pane: "accounts" })}>
            Open Settings
          </button>
        </div>
      );
    case "auth": {
      const keychain = authReason(banner.account) === "keychain";
      return (
        <div className="banner banner--error" role="alert">
          <Icon name="warning" size={14} />
          <span className="banner__text">{message}</span>
          <button
            type="button"
            className="banner__action"
            onClick={() =>
              onOpenSettings(
                keychain ? { pane: "accounts" } : { pane: "accounts", sheet: "replace", accountId: banner.account.id },
              )
            }
          >
            {keychain ? "Open Settings" : "Replace Token…"}
          </button>
        </div>
      );
    }
    case "config_error":
      return (
        <div className="banner banner--warn" role="status" title={banner.message}>
          <Icon name="warning" size={14} />
          {message}
        </div>
      );
    case "config_read_only":
    case "unreachable":
    case "rate_limited":
      return (
        <div className="banner banner--warn" role="status">
          <Icon name="warning" size={14} />
          {message}
        </div>
      );
    default:
      return null;
  }
}
