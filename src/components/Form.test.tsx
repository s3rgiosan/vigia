// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FormRow, FormSection, Toggle, useFormDescription } from "./Form";
import { PopUpButton } from "./PopUpButton";

afterEach(cleanup);

describe("FormSection", () => {
  it("renders the title, rows and footer", () => {
    render(
      <FormSection title="General" footer={<span>Note</span>}>
        <p>row</p>
      </FormSection>,
    );
    expect(screen.getByRole("heading", { name: "General" })).toBeTruthy();
    expect(screen.getByText("row")).toBeTruthy();
    expect(screen.getByText("Note")).toBeTruthy();
  });

  it("gives the footer an id", () => {
    render(
      <FormSection footer="Note" footerId="note">
        <p>row</p>
      </FormSection>,
    );
    expect(screen.getByText("Note").id).toBe("note");
  });

  it("omits the title and footer when absent", () => {
    const { container } = render(
      <FormSection>
        <p>row</p>
      </FormSection>,
    );
    expect(container.querySelector(".form-section__title")).toBeNull();
    expect(container.querySelector(".form-section__footer")).toBeNull();
  });
});

describe("FormRow", () => {
  it("labels the control when htmlFor is given", () => {
    render(
      <FormRow label="Name" htmlFor="name" help="Shown in the list">
        <input id="name" />
      </FormRow>,
    );
    expect(screen.getByLabelText("Name")).toBeTruthy();
    expect(screen.getByText("Shown in the list")).toBeTruthy();
  });

  it("passes the description id to a control rendered by a function", () => {
    render(
      <FormRow label="Name" htmlFor="name" description="Explains the field">
        {(describedBy) => <input id="name" aria-describedby={describedBy} />}
      </FormRow>,
    );
    const description = screen.getByText("Explains the field");
    expect(description.id).toBe("name-description");
    expect(screen.getByLabelText("Name").getAttribute("aria-describedby")).toBe("name-description");
  });

  it("describes a Toggle and a PopUpButton through context", () => {
    render(
      <>
        <FormRow label="Notify" htmlFor="notify" description="Sends a notification">
          <Toggle id="notify" label="Notify" checked={false} onChange={vi.fn()} />
        </FormRow>
        <FormRow label="Every" htmlFor="every" description="How often">
          <PopUpButton id="every" label="Every" value={1} options={[{ value: 1, label: "One" }]} onChange={vi.fn()} />
        </FormRow>
      </>,
    );
    expect(screen.getByRole("checkbox", { name: "Notify" }).getAttribute("aria-describedby")).toBe("notify-description");
    expect(screen.getByRole("combobox", { name: "Every" }).getAttribute("aria-describedby")).toBe("every-description");
  });

  it("gives controls no description outside a described row", () => {
    function Probe() {
      return <span>{useFormDescription() ?? "none"}</span>;
    }
    render(
      <FormRow label="Plain" htmlFor="plain">
        <Probe />
      </FormRow>,
    );
    expect(screen.getByText("none")).toBeTruthy();
  });

  it("renders a description without an id when there is no htmlFor", () => {
    render(<FormRow label="Static" description="Just text" />);
    expect(screen.getByText("Just text").id).toBe("");
  });

  it("uses a plain span label without htmlFor and skips an empty control", () => {
    const { container } = render(<FormRow label="Static" />);
    expect(container.querySelector("label")).toBeNull();
    expect(screen.getByText("Static").tagName).toBe("SPAN");
    expect(container.querySelector(".form-row__help")).toBeNull();
    expect(container.querySelector(".form-row__control")).toBeNull();
  });
});

describe("Toggle", () => {
  it("reports the new checked state", () => {
    const onChange = vi.fn();
    render(<Toggle checked={false} onChange={onChange} label="Notify" id="notify" />);
    fireEvent.click(screen.getByLabelText("Notify"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("renders disabled and checked", () => {
    render(<Toggle checked disabled onChange={vi.fn()} label="Notify" />);
    const box = screen.getByLabelText("Notify") as HTMLInputElement;
    expect(box.disabled).toBe(true);
    expect(box.checked).toBe(true);
  });
});
