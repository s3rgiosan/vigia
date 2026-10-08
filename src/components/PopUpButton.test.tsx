// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
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
  it("runs the chosen action and stays on its placeholder", () => {
    const removeAll = vi.fn();
    render(<ActionPopUpButton actions={[{ label: "Remove All…", onSelect: removeAll }]} />);
    const menu = screen.getByRole("combobox", { name: "More Actions" }) as HTMLSelectElement;
    fireEvent.change(menu, { target: { value: "0" } });
    expect(removeAll).toHaveBeenCalledTimes(1);
    expect(menu.value).toBe("");
  });

  it("is disabled when asked or when every action is unavailable", () => {
    const { rerender } = render(<ActionPopUpButton label="Actions" disabled actions={[{ label: "Go", onSelect: vi.fn() }]} />);
    expect((screen.getByLabelText("Actions") as HTMLSelectElement).disabled).toBe(true);
    rerender(<ActionPopUpButton label="Actions" actions={[{ label: "Go", onSelect: vi.fn(), disabled: true }]} />);
    expect((screen.getByLabelText("Actions") as HTMLSelectElement).disabled).toBe(true);
  });

  it("ignores a value that names no action", () => {
    const go = vi.fn();
    render(<ActionPopUpButton actions={[{ label: "Go", onSelect: go }]} />);
    fireEvent.change(screen.getByLabelText("More Actions"), { target: { value: "" } });
    expect(go).not.toHaveBeenCalled();
  });
});
