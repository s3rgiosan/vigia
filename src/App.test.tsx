// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import App from "./App";

vi.mock("./App.css", () => ({}));
vi.mock("./popup/Popup", () => ({ Popup: () => <div>popup view</div> }));
vi.mock("./settings/Settings", () => ({ Settings: () => <div>settings view</div> }));
vi.mock("./dev/Gallery", () => ({ Gallery: () => <div>gallery view</div> }));

afterEach(() => {
  cleanup();
  window.history.replaceState(null, "", "/");
});

function visit(search: string) {
  window.history.replaceState(null, "", `/${search}`);
}

describe("App", () => {
  it("shows the popup without a view parameter", async () => {
    render(<App />);
    expect(await screen.findByText("popup view")).toBeTruthy();
  });

  it("shows the settings window for view=settings", async () => {
    visit("?view=settings");
    render(<App />);
    expect(await screen.findByText("settings view")).toBeTruthy();
  });

  it("falls back to the popup for an unknown view", async () => {
    visit("?view=nonsense");
    render(<App />);
    expect(await screen.findByText("popup view")).toBeTruthy();
  });

  it("shows the control gallery for view=gallery on the dev server", async () => {
    visit("?view=gallery");
    render(<App />);
    expect(await screen.findByText("gallery view")).toBeTruthy();
  });

  it("ignores view=gallery outside the dev server", async () => {
    vi.stubEnv("DEV", false);
    vi.resetModules();
    const { default: ProdApp } = await import("./App");
    visit("?view=gallery");
    render(<ProdApp />);
    expect(await screen.findByText("popup view")).toBeTruthy();
    vi.unstubAllEnvs();
  });
});
