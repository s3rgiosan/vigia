// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

afterEach(cleanup);

let mounts = 0;
let shouldThrow = true;

function Bomb() {
  if (shouldThrow) {
    throw new Error("boom");
  }
  return <span>fine</span>;
}

function Counter() {
  mounts += 1;
  return <span>counter</span>;
}

describe("ErrorBoundary", () => {
  it("renders its children when nothing fails", () => {
    render(
      <ErrorBoundary>
        <span>content</span>
      </ErrorBoundary>,
    );
    expect(screen.getByText("content")).toBeTruthy();
  });

  it("shows the fallback, logs the error and remounts the subtree on reload", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    shouldThrow = true;
    render(
      <ErrorBoundary>
        <Bomb />
      </ErrorBoundary>,
    );
    expect(screen.getByRole("alert").textContent).toContain("Something went wrong in this view.");
    expect(error.mock.calls.some((c) => c[0] === "view crashed")).toBe(true);

    shouldThrow = false;
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    expect(screen.getByText("fine")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
    error.mockRestore();
  });

  it("mounts the children fresh after a reload", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    mounts = 0;
    shouldThrow = true;
    function Both() {
      return (
        <>
          <Counter />
          <Bomb />
        </>
      );
    }
    render(
      <ErrorBoundary>
        <Both />
      </ErrorBoundary>,
    );
    const before = mounts;
    shouldThrow = false;
    fireEvent.click(screen.getByRole("button", { name: "Reload" }));
    expect(mounts).toBeGreaterThan(before);
    error.mockRestore();
  });
});
