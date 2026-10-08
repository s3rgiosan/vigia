import { useFormDescription } from "./Form";

function Chevron() {
  return (
    <svg className="popup-button__chevron" width="8" height="12" viewBox="0 0 8 12" aria-hidden="true">
      <path d="M1.2 4.3 4 1.5l2.8 2.8M1.2 7.7 4 10.5l2.8-2.8" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

/**
 * A macOS pop-up button: a filled button showing the current choice and a ⌃⌄ chevron. A
 * transparent native <select> covers it, so clicking opens WebKit's native menu with the system
 * look, keyboard handling and checkmark on the current item.
 */
export function PopUpButton<T extends string | number>({
  value,
  options,
  onChange,
  disabled,
  id,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
  disabled?: boolean;
  id?: string;
  label: string;
}) {
  const describedBy = useFormDescription();
  const current = options.find((o) => o.value === value)?.label ?? "";
  return (
    <span className={`popup-button ${disabled ? "popup-button--disabled" : ""}`}>
      <span className="popup-button__label">{current}</span>
      <Chevron />
      <select
        id={id}
        aria-label={label}
        aria-describedby={describedBy}
        value={String(value)}
        disabled={disabled}
        onChange={(e) => {
          const picked = options.find((o) => String(o.value) === e.target.value);
          if (picked) {
            onChange(picked.value);
          }
        }}
      >
        {options.map((o) => (
          <option key={String(o.value)} value={String(o.value)}>
            {o.label}
          </option>
        ))}
      </select>
    </span>
  );
}

export interface PopUpAction {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
}

/**
 * An action pop-up button: a small "…" button whose native menu runs a command instead of
 * holding a value. The select stays on its hidden placeholder, so any item can be chosen again.
 */
export function ActionPopUpButton({
  actions,
  label = "More Actions",
  disabled,
}: {
  actions: PopUpAction[];
  label?: string;
  disabled?: boolean;
}) {
  const unavailable = disabled || actions.every((a) => a.disabled);
  return (
    <span className={`action-popup ${unavailable ? "action-popup--disabled" : ""}`}>
      <svg className="action-popup__glyph" width="13" height="3" viewBox="0 0 13 3" aria-hidden="true">
        <circle cx="1.5" cy="1.5" r="1.4" fill="currentColor" />
        <circle cx="6.5" cy="1.5" r="1.4" fill="currentColor" />
        <circle cx="11.5" cy="1.5" r="1.4" fill="currentColor" />
      </svg>
      <select
        aria-label={label}
        title={label}
        value=""
        disabled={unavailable}
        onChange={(e) => {
          const picked = e.target.value;
          actions.find((_, index) => String(index) === picked)?.onSelect();
        }}
      >
        <option value="" disabled hidden>
          {label}
        </option>
        {actions.map((a, index) => (
          <option key={a.label} value={index} disabled={a.disabled}>
            {a.label}
          </option>
        ))}
      </select>
    </span>
  );
}
