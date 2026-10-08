// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
// @ts-expect-error type error without @types/node package
import { readFileSync } from "node:fs";
import { Icon, SYMBOL_NAMES } from "./Icon";

afterEach(cleanup);

describe("Icon", () => {
  it("draws brand marks as an SVG of the requested size", () => {
    const { container } = render(<Icon name="github" size={20} className="brand" />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("width")).toBe("20");
    expect(svg?.getAttribute("class")).toBe("brand");
    expect(svg?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelector("path")?.getAttribute("d")).toBeTruthy();
  });

  it("uses a different path for each brand", () => {
    const github = render(<Icon name="github" />).container.querySelector("path")?.getAttribute("d");
    const gitlab = render(<Icon name="gitlab" />).container.querySelector("path")?.getAttribute("d");
    expect(github).not.toBe(gitlab);
  });

  it("draws symbols as a masked span with the default size", () => {
    const { container } = render(<Icon name="gear" />);
    const span = container.querySelector("span") as HTMLElement;
    expect(span.className).toBe("symbol symbol--gear");
    expect(span.style.width).toBe("16px");
    expect(span.getAttribute("aria-hidden")).toBe("true");
    expect(span.getAttribute("style")).not.toContain("mask-image");
  });

  it("adds the extra class to a symbol", () => {
    const { container } = render(<Icon name="refresh" size={12} className="spin" />);
    const span = container.querySelector("span") as HTMLElement;
    expect(span.className).toBe("symbol symbol--refresh spin");
    expect(span.style.height).toBe("12px");
  });

  it("renders every symbol with a mask class declared in symbols.css", () => {
    const css: string = readFileSync("src/assets/symbols/symbols.css", "utf8");
    for (const name of SYMBOL_NAMES) {
      const { container } = render(<Icon name={name} />);
      expect(container.querySelector(`.symbol.symbol--${name}`)).toBeTruthy();
      expect(css).toContain(`.symbol--${name} {`);
    }
  });
});
