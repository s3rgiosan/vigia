import { getVersion } from "@tauri-apps/api/app";
import { useEffect, useState } from "react";
import { FormRow, FormSection, Toggle } from "../components/Form";
import { PopUpButton } from "../components/PopUpButton";
import { useInstallUpdate } from "../lib/hooks/useInstallUpdate";
import type { RepoOrder, UpdateInfo } from "../lib/snapshot";
import { checkForUpdatesNow } from "../lib/tauri";
import { useSettings } from "./SettingsContext";

const INTERVALS: { secs: number; label: string }[] = [
  { secs: 15, label: "15 seconds" },
  { secs: 30, label: "30 seconds" },
  { secs: 60, label: "1 minute" },
  { secs: 120, label: "2 minutes" },
  { secs: 300, label: "5 minutes" },
  { secs: 600, label: "10 minutes" },
];

const ORDERS: { value: RepoOrder; label: string }[] = [
  { value: "name", label: "Name" },
  { value: "recent", label: "Most recent run" },
];

export function GeneralTab() {
  const { view, saveSettings: save, readOnly: locked } = useSettings();
  const s = view.settings;
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion()
      .then(setVersion)
      .catch(() => setVersion(""));
  }, []);
  const intervals = INTERVALS.some((i) => i.secs === s.poll_interval_secs)
    ? INTERVALS
    : [...INTERVALS, { secs: s.poll_interval_secs, label: `${s.poll_interval_secs} seconds` }].sort((a, b) => a.secs - b.secs);

  return (
    <div className="form-pane">
      <FormSection
        title="Checking"
        footer="Repositories with a running job are checked every 15 seconds. Vigia slows down on its own to stay within the API rate limit."
      >
        <FormRow label="Check every" htmlFor="interval">
          <PopUpButton
            id="interval"
            label="Check every"
            value={s.poll_interval_secs}
            onChange={(secs) => save({ poll_interval_secs: secs })}
            disabled={locked}
            options={intervals.map((i) => ({ value: i.secs, label: i.label }))}
          />
        </FormRow>
        <FormRow label="Ignore pull request runs" help="Skips runs triggered by pull requests and merge requests.">
          <Toggle
            label="Ignore pull request runs"
            checked={s.exclude_pull_requests}
            onChange={(v) => save({ exclude_pull_requests: v })}
            disabled={locked}
          />
        </FormRow>
      </FormSection>

      <FormSection title="Repository list" footer="Status sections stay in place; the order applies within each one.">
        <FormRow label="Sort by" htmlFor="repo-order">
          <PopUpButton
            id="repo-order"
            label="Sort repositories by"
            value={s.repo_order}
            onChange={(order) => save({ repo_order: order })}
            disabled={locked}
            options={ORDERS}
          />
        </FormRow>
        <FormRow label="Group by organization">
          <Toggle
            label="Group repositories by organization"
            checked={s.group_by_org}
            onChange={(v) => save({ group_by_org: v })}
            disabled={locked}
          />
        </FormRow>
      </FormSection>

      <FormSection title="Notifications">
        <FormRow label="When a branch fails">
          <Toggle
            label="Notify when a branch fails"
            checked={s.notify_failures}
            onChange={(v) => save({ notify_failures: v })}
            disabled={locked}
          />
        </FormRow>
        <FormRow label="When it recovers">
          <Toggle
            label="Notify when a branch recovers"
            checked={s.notify_recoveries}
            onChange={(v) => save({ notify_recoveries: v })}
            disabled={locked || !s.notify_failures}
          />
        </FormRow>
      </FormSection>

      <FormSection title="Startup">
        <FormRow label="Open at login">
          <Toggle
            label="Open Vigia at login"
            checked={s.launch_at_login}
            onChange={(v) => save({ launch_at_login: v })}
            disabled={locked}
          />
        </FormRow>
      </FormSection>

      <FormSection title="Updates">
        <FormRow label="Check for updates automatically">
          <Toggle
            label="Check for updates automatically"
            checked={s.check_for_updates}
            onChange={(v) => save({ check_for_updates: v })}
            disabled={locked}
          />
        </FormRow>
        <FormRow label="Version">
          <span className="value">{version}</span>
        </FormRow>
        <FormRow label="Check now">
          <UpdateCheck />
        </FormRow>
      </FormSection>
    </div>
  );
}

type CheckState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "current" }
  | { kind: "available"; update: UpdateInfo }
  | { kind: "failed" }
  | { kind: "install_failed" };

/** The Check Now button with its one-line result, or the install button once a release is found. */
function UpdateCheck() {
  const [state, setState] = useState<CheckState>({ kind: "idle" });
  const { busy, label, install } = useInstallUpdate(() => setState({ kind: "install_failed" }));

  function check() {
    setState({ kind: "checking" });
    checkForUpdatesNow()
      .then((update) => setState(update ? { kind: "available", update } : { kind: "current" }))
      .catch((e) => {
        console.error(e);
        setState({ kind: "failed" });
      });
  }

  let dot: string | null = null;
  let text = "";
  switch (state.kind) {
    case "checking":
      dot = "gray";
      text = "Checking…";
      break;
    case "current":
      dot = "green";
      text = "Up to date";
      break;
    case "available":
      dot = "green";
      text = `Version ${state.update.version} available`;
      break;
    case "failed":
      dot = "red";
      text = "Couldn’t check for updates";
      break;
    case "install_failed":
      dot = "red";
      text = "Couldn’t install the update";
      break;
    default:
      break;
  }

  return (
    <>
      <span className="value value--status" role="status">
        {dot ? <span className={`dot dot--${dot}`} aria-hidden="true" /> : null}
        {text}
      </span>
      {state.kind === "available" ? (
        <button type="button" onClick={install} disabled={busy}>
          {label}
        </button>
      ) : (
        <button type="button" onClick={check} disabled={state.kind === "checking"}>
          Check Now
        </button>
      )}
    </>
  );
}
