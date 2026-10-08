import { createContext, useContext, type ReactNode } from "react";

/** The id of the description of the enclosing FormRow, when it has one. */
const DescriptionContext = createContext<string | undefined>(undefined);

/**
 * The value for a control's `aria-describedby` inside a FormRow with a description. Controls
 * outside such a row get undefined.
 */
export function useFormDescription(): string | undefined {
  return useContext(DescriptionContext);
}

/** A grouped form section: header above, rounded box of rows, optional footnote below. */
export function FormSection({
  title,
  footer,
  footerId,
  children,
}: {
  title?: string;
  footer?: ReactNode;
  footerId?: string;
  children: ReactNode;
}) {
  return (
    <section className="form-section">
      {title ? <h3 className="form-section__title">{title}</h3> : null}
      <div className="form-group">{children}</div>
      {footer ? <div className="form-section__footer" id={footerId}>{footer}</div> : null}
    </section>
  );
}

/**
 * One row: label on the left, control on the right, optional help and description under the
 * label. With `htmlFor` and a description, the description id is `<htmlFor>-description`; controls
 * read it through `useFormDescription`, or `children` can be a function that receives it.
 */
export function FormRow({
  label,
  help,
  description,
  children,
  htmlFor,
}: {
  label: ReactNode;
  help?: ReactNode;
  description?: ReactNode;
  children?: ReactNode | ((describedBy: string | undefined) => ReactNode);
  htmlFor?: string;
}) {
  const descriptionId = htmlFor && description ? `${htmlFor}-description` : undefined;
  const control = typeof children === "function" ? children(descriptionId) : children;

  return (
    <div className="form-row">
      <div className="form-row__label">
        {htmlFor ? <label htmlFor={htmlFor}>{label}</label> : <span>{label}</span>}
        {help ? <span className="form-row__help">{help}</span> : null}
        {description ? (
          <span className="form-row__description" id={descriptionId}>
            {description}
          </span>
        ) : null}
      </div>
      {control ? (
        <div className="form-row__control">
          <DescriptionContext.Provider value={descriptionId}>{control}</DescriptionContext.Provider>
        </div>
      ) : null}
    </div>
  );
}

/** A checkbox drawn as an AppKit switch (see `.toggle` in App.css). */
export function Toggle({
  checked,
  onChange,
  disabled,
  label,
  id,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  label: string;
  id?: string;
}) {
  const describedBy = useFormDescription();
  return (
    <input
      id={id}
      type="checkbox"
      className="toggle"
      checked={checked}
      disabled={disabled}
      aria-label={label}
      aria-describedby={describedBy}
      onChange={(e) => onChange(e.target.checked)}
    />
  );
}
