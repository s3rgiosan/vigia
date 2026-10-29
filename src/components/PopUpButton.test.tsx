// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

const showActionMenu = vi.fn();
vi.mock("../lib/actionMenu", () => ({
  showActionMenu: (...a: unknown[]) => showActionMenu(...a),
}));

import { ActionPopUpButton, PopUpButton } from "./PopUpButton";

afterEach(cleanup);

const OPTIONS = [
  { value: 30, label: "30 seconds" },
  { value: 60, label: "1 minute" },
];

describe("PopUpButton", () => {
  it("shows the label of the current value and lists every option", () => {
    const { container } = render(<PopUpButton value={60} options={OPTIONS} onChange={vi.fn()} label="Interval" />);
    expect(container.querySelector(".popup-button__label")?.textContent).toBe("1 minute");
    expect(screen.getAllByRole("option")).toHaveLength(2);
    expect((screen.getByLabelText("Interval") as HTMLSelectElement).value).toBe("60");
  });

  it("shows an empty label when the value matches no option", () => {
    const { container } = render(<PopUpButton value={5} options={OPTIONS} onChange={vi.fn()} label="Interval" />);
    expect(container.querySelector(".popup-button__label")?.textContent).toBe("");
  });

  it("passes the typed value of the picked option", () => {
    const onChange = vi.fn();
    render(<PopUpButton value={60} options={OPTIONS} onChange={onChange} label="Interval" id="interval" />);
    fireEvent.change(screen.getByLabelText("Interval"), { target: { value: "30" } });
    expect(onChange).toHaveBeenCalledWith(30);
  });

  it("works with string values", () => {
    const onChange = vi.fn();
    render(
      <PopUpButton
        value="a"
        options={[
          { value: "a", label: "Alpha" },
          { value: "b", label: "Beta" },
        ]}
        onChange={onChange}
        label="Letter"
      />,
    );
    fireEvent.change(screen.getByLabelText("Letter"), { target: { value: "b" } });
    expect(onChange).toHaveBeenCalledWith("b");
  });

  it("ignores a change to a value that is not an option", () => {
    const onChange = vi.fn();
    render(<PopUpButton value={60} options={OPTIONS} onChange={onChange} label="Interval" />);
    const select = screen.getByLabelText("Interval") as HTMLSelectElement;
    const stray = document.createElement("option");
    stray.value = "999";
    select.appendChild(stray);
    fireEvent.change(select, { target: { value: "999" } });
    expect(onChange).not.toHaveBeenCalled();
  });

  it("marks itself and the select disabled", () => {
    const { container } = render(<PopUpButton value={60} options={OPTIONS} onChange={vi.fn()} label="Interval" disabled />);
    expect(container.querySelector(".popup-button--disabled")).toBeTruthy();
    expect((screen.getByLabelText("Interval") as HTMLSelectElement).disabled).toBe(true);
  });
});

describe("ActionPopUpButton", () => {
  it("opens a native menu of its actions under the button", () => {
    showActionMenu.mockResolvedValue(undefined);
    const actions = [{ label: "Remove All…", onSelect: vi.fn() }];
    render(<ActionPopUpButton actions={actions} />);
    const button = screen.getByRole("button", { name: "More Actions" });
    expect(button.getAttribute("aria-haspopup")).toBe("menu");
    fireEvent.click(button);
    expect(showActionMenu).toHaveBeenCalledWith(actions, { x: 0, y: 4 });
  });

  it("is disabled when asked or when every action is unavailable", () => {
    const { rerender } = render(<ActionPopUpButton label="Actions" disabled actions={[{ label: "Go", onSelect: vi.fn() }]} />);
    expect((screen.getByRole("button", { name: "Actions" }) as HTMLButtonElement).disabled).toBe(true);
    rerender(<ActionPopUpButton label="Actions" actions={[{ label: "Go", onSelect: vi.fn(), disabled: true }]} />);
    expect((screen.getByRole("button", { name: "Actions" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("logs a menu that fails to open", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    showActionMenu.mockRejectedValue(new Error("no menu"));
    render(<ActionPopUpButton actions={[{ label: "Go", onSelect: vi.fn() }]} />);
    fireEvent.click(screen.getByRole("button", { name: "More Actions" }));
    await Promise.resolve();
    await Promise.resolve();
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});
