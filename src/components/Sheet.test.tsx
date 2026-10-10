// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setSettingsToolbarEnabled = vi.fn();

vi.mock("../lib/tauri", () => ({
  setSettingsToolbarEnabled: (enabled: boolean) => setSettingsToolbarEnabled(enabled),
}));

import { OpenSheetsContext, Sheet, SheetNote } from "./Sheet";

beforeEach(() => {
  setSettingsToolbarEnabled.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__;
});

function Parent({ busy = false, onCancel }: { busy?: boolean; onCancel: () => void }) {
  const [count, setCount] = useState(0);
  return (
    <>
      <button type="button" onClick={() => setCount(count + 1)}>
        rerender {count}
      </button>
      <Sheet title="Test" submitLabel="Go" busy={busy} onCancel={() => onCancel()} onSubmit={() => undefined}>
        <input aria-label="first" />
        <input aria-label="token" />
      </Sheet>
    </>
  );
}

describe("Sheet", () => {
  it("focuses the first input on mount", () => {
    render(<Parent onCancel={() => undefined} />);
    expect(document.activeElement).toBe(screen.getByLabelText("first"));
  });

  it("keeps focus in a typed input across a parent re-render", () => {
    render(<Parent onCancel={() => undefined} />);
    const token = screen.getByLabelText("token") as HTMLInputElement;
    token.focus();
    fireEvent.change(token, { target: { value: "github_pat_abc" } });
    act(() => {
      screen.getByText("rerender 0").click();
    });
    expect(screen.getByText("rerender 1")).toBeTruthy();
    expect(document.activeElement).toBe(token);
    expect(token.value).toBe("github_pat_abc");
  });

  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    render(<Parent onCancel={onCancel} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("ignores Escape while busy", () => {
    const onCancel = vi.fn();
    render(<Parent busy onCancel={onCancel} />);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("wraps Tab from the last control to the first", () => {
    render(<Parent onCancel={() => undefined} />);
    const submit = screen.getByText("Go");
    submit.focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByLabelText("first"));
  });

  it("pulls focus back inside when it is outside the sheet", () => {
    render(<Parent onCancel={() => undefined} />);
    screen.getByText("rerender 0").focus();
    fireEvent.keyDown(window, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByLabelText("first"));
  });

  describe("destructive", () => {
    function renderDestructive(onCancel = vi.fn(), onSubmit = vi.fn()) {
      render(
        <Sheet title="Remove" submitLabel="Remove" destructive onCancel={onCancel} onSubmit={onSubmit}>
          <p>Are you sure?</p>
        </Sheet>,
      );
      return { onCancel, onSubmit };
    }

    it("is an alert dialog described by its message", () => {
      render(
        <Sheet title="Remove" message="This cannot be undone." submitLabel="Remove" destructive onCancel={vi.fn()} onSubmit={vi.fn()} />,
      );
      const alert = screen.getByRole("alertdialog", { name: "Remove" });
      const id = alert.getAttribute("aria-describedby") as string;
      expect(document.getElementById(id)?.textContent).toBe("This cannot be undone.");
    });

    it("makes Cancel the default button and gives it focus", () => {
      renderDestructive();
      const cancel = screen.getByText("Cancel");
      expect(cancel.className).toBe("default");
      expect(document.activeElement).toBe(cancel);
      expect(screen.getByRole("button", { name: "Remove" }).className).toBe("destructive");
    });

    it("cancels on Return and does not confirm", () => {
      const { onCancel, onSubmit } = renderDestructive();
      fireEvent.keyDown(screen.getByText("Are you sure?"), { key: "Enter" });
      expect(onCancel).toHaveBeenCalledTimes(1);
      expect(onSubmit).not.toHaveBeenCalled();
    });

    it("confirms when the destructive button is clicked", () => {
      const { onSubmit } = renderDestructive();
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
      expect(onSubmit).toHaveBeenCalledTimes(1);
    });
  });

  it("keeps the submit button as the default on a normal sheet", () => {
    render(<Parent onCancel={() => undefined} />);
    expect(screen.getByText("Go").className).toBe("default");
    expect(screen.getByText("Cancel").className).toBe("");
  });

  it("shows the busy label", () => {
    render(
      <Sheet title="T" submitLabel="Go" busy busyLabel="Adding…" onCancel={() => undefined} onSubmit={() => undefined}>
        <input aria-label="x" />
      </Sheet>,
    );
    expect(screen.getByText("Adding…")).toBeTruthy();
  });
});

describe("Sheet keyboard and wiring", () => {
  function renderSheet(extra: Partial<React.ComponentProps<typeof Sheet>> = {}) {
    return render(
      <>
        <button type="button">outside</button>
        <Sheet title="T" submitLabel="Go" onCancel={() => undefined} onSubmit={() => undefined} {...extra}>
          <input aria-label="first" />
          <input aria-label="second" disabled />
        </Sheet>
      </>,
    );
  }

  it("wraps Shift+Tab from the first control to the last", () => {
    renderSheet();
    (screen.getByLabelText("first") as HTMLElement).focus();
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByText("Go"));
  });

  it("pulls focus to the last control on Shift+Tab from outside", () => {
    renderSheet();
    screen.getByText("outside").focus();
    fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByText("Go"));
  });

  it("lets Tab move normally between inner controls", () => {
    renderSheet();
    (screen.getByLabelText("first") as HTMLElement).focus();
    const notPrevented = fireEvent.keyDown(window, { key: "Tab" });
    expect(notPrevented).toBe(true);
    const shiftOnMiddle = fireEvent.keyDown(window, { key: "Tab", shiftKey: true });
    expect(shiftOnMiddle).toBe(false);
  });

  it("ignores other keys", () => {
    renderSheet();
    expect(fireEvent.keyDown(window, { key: "a" })).toBe(true);
  });

  it("keeps Tab inside when nothing in the sheet can take focus", () => {
    render(
      <Sheet title="T" submitLabel="Go" busy onCancel={() => undefined} onSubmit={() => undefined}>
        <p>wait</p>
      </Sheet>,
    );
    expect(fireEvent.keyDown(window, { key: "Tab" })).toBe(false);
  });

  it("submits through the form unless disabled", () => {
    const onSubmit = vi.fn();
    const { rerender } = renderSheet({ onSubmit });
    fireEvent.submit(screen.getByRole("dialog"));
    expect(onSubmit).toHaveBeenCalledTimes(1);
    rerender(
      <Sheet title="T" submitLabel="Go" submitDisabled onCancel={() => undefined} onSubmit={onSubmit}>
        <input aria-label="first" />
      </Sheet>,
    );
    fireEvent.submit(screen.getByRole("dialog"));
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("does not cancel a destructive sheet on Return while busy", () => {
    const onCancel = vi.fn();
    render(
      <Sheet title="R" submitLabel="Remove" destructive busy onCancel={onCancel} onSubmit={() => undefined}>
        <p>Sure?</p>
      </Sheet>,
    );
    fireEvent.keyDown(screen.getByText("Sure?"), { key: "Enter" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("lets Return activate a focused button on a destructive sheet", () => {
    const onCancel = vi.fn();
    render(
      <Sheet title="R" submitLabel="Remove" destructive onCancel={onCancel} onSubmit={() => undefined}>
        <p>Sure?</p>
      </Sheet>,
    );
    fireEvent.keyDown(screen.getByRole("button", { name: "Remove" }), { key: "Enter" });
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("shows the accessory and applies the width", () => {
    renderSheet({ accessory: <button type="button">Test</button>, width: 300 });
    expect(screen.getByText("Test")).toBeTruthy();
    expect((screen.getByRole("dialog") as HTMLElement).style.width).toBe("300px");
  });

  it("sizes a fitting sheet to its content within the window", () => {
    renderSheet({ width: "fit" });
    const { style } = screen.getByRole("dialog") as HTMLElement;
    expect(style.width).toBe("max-content");
    expect(style.minWidth).toBe("440px");
    expect(style.maxWidth).toBe("calc(100vw - 48px)");
  });

  it("has no description without a message", () => {
    renderSheet();
    expect(screen.getByRole("dialog").hasAttribute("aria-describedby")).toBe(false);
  });
});

describe("Sheet modality", () => {
  function Window({ open }: { open: boolean }) {
    return (
      <div>
        <button type="button">opener</button>
        <section>
          <p>pane text</p>
          {open ? (
            <Sheet title="T" submitLabel="Go" onCancel={() => undefined} onSubmit={() => undefined}>
              <input aria-label="inside" />
            </Sheet>
          ) : null}
        </section>
      </div>
    );
  }

  it("makes the rest of the window inert while open and restores it", () => {
    const { rerender } = render(<Window open={false} />);
    const preInert = document.createElement("aside");
    preInert.setAttribute("inert", "");
    document.body.appendChild(preInert);
    rerender(<Window open />);
    expect(screen.getByText("opener").hasAttribute("inert")).toBe(true);
    expect(screen.getByText("pane text").hasAttribute("inert")).toBe(true);
    expect(screen.getByLabelText("inside").closest("[inert]")).toBeNull();
    rerender(<Window open={false} />);
    expect(screen.getByText("opener").hasAttribute("inert")).toBe(false);
    expect(screen.getByText("pane text").hasAttribute("inert")).toBe(false);
    expect(preInert.hasAttribute("inert")).toBe(true);
    preInert.remove();
  });

  it("returns focus to the element focused before it opened", () => {
    const { rerender } = render(<Window open={false} />);
    screen.getByText("opener").focus();
    rerender(<Window open />);
    expect(document.activeElement).toBe(screen.getByLabelText("inside"));
    rerender(<Window open={false} />);
    expect(document.activeElement).toBe(screen.getByText("opener"));
  });
});

function Toggle() {
  const [open, setOpen] = useState(true);
  return (
    <div>
      {open ? <Sheet title="Closing" submitLabel="Go" onCancel={() => setOpen(false)} onSubmit={() => undefined} /> : null}
    </div>
  );
}

describe("sheet exit", () => {
  function animated(name: string) {
    const real = window.getComputedStyle;
    return vi.spyOn(window, "getComputedStyle").mockImplementation((el, pseudo) => {
      const style = real(el, pseudo);
      return { ...style, animationName: name } as CSSStyleDeclaration;
    });
  }

  it("plays the exit on an inert copy and removes it when the animation ends", async () => {
    const style = animated("sheet-out");
    render(<Toggle />);
    fireEvent.keyDown(window, { key: "Escape" });
    await act(async () => undefined);
    const ghost = document.querySelector<HTMLElement>(".sheet-backdrop--closing");
    expect(ghost).not.toBeNull();
    expect(ghost?.hasAttribute("inert")).toBe(true);
    expect(ghost?.getAttribute("aria-hidden")).toBe("true");
    expect(screen.queryByRole("dialog")).toBeNull();
    fireEvent.animationEnd(ghost as HTMLElement);
    expect(document.querySelector(".sheet-backdrop--closing")).toBeNull();
    style.mockRestore();
  });

  it("removes the copy after a fallback delay when no animation end arrives", async () => {
    vi.useFakeTimers();
    const style = animated("sheet-out");
    render(<Toggle />);
    fireEvent.keyDown(window, { key: "Escape" });
    await act(async () => undefined);
    expect(document.querySelector(".sheet-backdrop--closing")).not.toBeNull();
    act(() => vi.advanceTimersByTime(400));
    expect(document.querySelector(".sheet-backdrop--closing")).toBeNull();
    style.mockRestore();
    vi.useRealTimers();
  });

  it("leaves no copy when the styles give it no animation", async () => {
    const style = animated("none");
    render(<Toggle />);
    fireEvent.keyDown(window, { key: "Escape" });
    await act(async () => undefined);
    expect(document.querySelector(".sheet-backdrop")).toBeNull();
    style.mockRestore();
  });

  it("plays no exit when the whole window goes away", async () => {
    const style = animated("sheet-out");
    const { unmount } = render(<Toggle />);
    unmount();
    await act(async () => undefined);
    expect(document.querySelector(".sheet-backdrop--closing")).toBeNull();
    style.mockRestore();
  });
});

describe("open sheet count", () => {
  function Sheets({ count }: { count: number }) {
    return (
      <>
        {Array.from({ length: count }, (_, i) => (
          <Sheet key={i} title={`T${i}`} submitLabel="Go" onCancel={() => undefined} onSubmit={() => undefined} />
        ))}
      </>
    );
  }

  it("counts open sheets and disables the native toolbar while any is open", () => {
    const counter = { current: 0 };
    const ui = (count: number) => (
      <OpenSheetsContext.Provider value={counter}>
        <Sheets count={count} />
      </OpenSheetsContext.Provider>
    );
    const { rerender, unmount } = render(ui(1));
    expect(counter.current).toBe(1);
    expect(setSettingsToolbarEnabled.mock.calls).toEqual([[false]]);
    rerender(ui(2));
    expect(counter.current).toBe(2);
    rerender(ui(1));
    expect(setSettingsToolbarEnabled.mock.calls).toEqual([[false]]);
    unmount();
    expect(counter.current).toBe(0);
    expect(setSettingsToolbarEnabled.mock.calls).toEqual([[false], [true]]);
  });

  it("leaves the toolbar alone without a counter or in the browser preview", () => {
    const { unmount } = render(<Sheets count={1} />);
    unmount();
    (window as unknown as Record<string, unknown>).__VIGIA_PREVIEW__ = true;
    const counter = { current: 0 };
    const preview = render(
      <OpenSheetsContext.Provider value={counter}>
        <Sheets count={1} />
      </OpenSheetsContext.Provider>,
    );
    preview.unmount();
    expect(setSettingsToolbarEnabled).not.toHaveBeenCalled();
  });

  it("logs a toolbar call that fails", async () => {
    const log = vi.spyOn(console, "error").mockImplementation(() => undefined);
    setSettingsToolbarEnabled.mockRejectedValue(new Error("no window"));
    render(
      <OpenSheetsContext.Provider value={{ current: 0 }}>
        <Sheets count={1} />
      </OpenSheetsContext.Provider>,
    );
    await act(async () => undefined);
    expect(log).toHaveBeenCalled();
    log.mockRestore();
  });
});

describe("SheetNote", () => {
  it("announces errors and shows warnings quietly", () => {
    render(
      <>
        <SheetNote>Bad token</SheetNote>
        <SheetNote tone="warning">Careful</SheetNote>
      </>,
    );
    expect(screen.getByRole("alert").textContent).toBe("Bad token");
    expect(screen.getByText("Careful").closest("p")?.getAttribute("role")).toBeNull();
  });
});
