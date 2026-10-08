import { Icon } from "../components/Icon";
import { useInstallUpdate } from "../lib/hooks/useInstallUpdate";
import type { UpdateInfo } from "../lib/snapshot";

/** Announces an available release and installs it on request. */
export function UpdateBanner({ update, onError }: { update: UpdateInfo; onError: (e: unknown) => void }) {
  const { busy, label, install } = useInstallUpdate(onError);
  return (
    <div className="banner banner--info" role="status">
      <Icon name="refresh" size={14} />
      <span className="banner__text">{`Vigia ${update.version} is available.`}</span>
      <button type="button" className="banner__action" onClick={install} disabled={busy}>
        {label}
      </button>
    </div>
  );
}
