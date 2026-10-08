import { useEffect, useState } from "react";
import { useFormDescription } from "../components/Form";
import { useLatest } from "../lib/hooks/useLatest";
import { parsePatterns, samePatterns } from "./filters";

/** Typed text and the saved value it was typed against. */
interface Draft {
  text: string;
  base: string;
  /** True until the text is committed. */
  dirty: boolean;
}

/**
 * A text field for a list of globs. It saves when it loses focus, Return is pressed or it
 * unmounts mid-edit, and only when the list changed. A committed draft shows until the saved value
 * changes; while nothing is being typed the field follows `value`.
 */
export function PatternField({
  value,
  onSave,
  placeholder,
  disabled,
  id,
  wide,
  label,
}: {
  value: string[];
  onSave: (patterns: string[]) => void;
  placeholder: string;
  disabled: boolean;
  id?: string;
  wide?: boolean;
  label?: string;
}) {
  const describedBy = useFormDescription();
  const saved = value.join(", ");
  const [draft, setDraft] = useState<Draft | null>(null);
  const text = draft && (draft.dirty || draft.base === saved) ? draft.text : saved;
  const latest = useLatest({ draft, value, onSave });

  function commit() {
    const current = latest.current;
    if (!current.draft?.dirty) {
      return;
    }
    const next = parsePatterns(current.draft.text);
    setDraft({ text: next.join(", "), base: saved, dirty: false });
    if (!samePatterns(next, current.value)) {
      current.onSave(next);
    }
  }

  useEffect(() => {
    return () => {
      const current = latest.current;
      if (current.draft?.dirty) {
        const next = parsePatterns(current.draft.text);
        if (!samePatterns(next, current.value)) {
          current.onSave(next);
        }
      }
    };
  }, [latest]);

  return (
    <input
      id={id}
      aria-label={label}
      aria-describedby={describedBy}
      className={`field ${wide ? "field--wide" : ""}`}
      value={text}
      onChange={(e) => setDraft({ text: e.target.value, base: saved, dirty: true })}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.currentTarget.blur();
        }
      }}
      placeholder={placeholder}
      title={placeholder}
      disabled={disabled}
      spellCheck={false}
    />
  );
}
