// @vitest-environment jsdom
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Organization, OrgFilters, SettingsView, WatchedRepo } from "../lib/tauri";
import { makeOrganization, makeSettings, makeSettingsAccount, makeView, makeWatchedRepo } from "../test/fixtures";
import { renderWithSettings } from "../test/settings";

const clearRepoOverrides = vi.fn();
const setOrgFilters = vi.fn();
const setRepoBranches = vi.fn();
const setRepoIgnoredWorkflows = vi.fn();
const setRepoIncludeTags = vi.fn();

vi.mock("../lib/tauri", () => ({
  clearRepoOverrides: (...args: unknown[]) => clearRepoOverrides(...args),
  setOrgFilters: (...args: unknown[]) => setOrgFilters(...args),
  setRepoBranches: (...args: unknown[]) => setRepoBranches(...args),
  setRepoIncludeTags: (...args: unknown[]) => setRepoIncludeTags(...args),
  setRepoIgnoredWorkflows: (...args: unknown[]) => setRepoIgnoredWorkflows(...args),
}));

import { BranchesTab } from "./BranchesTab";

const SCOPE_KEY = "vigia.settings.filters.scope";
const NONE: OrgFilters = { branch_patterns: null, ignored_workflows: null, include_tags: null };

beforeEach(() => {
  localStorage.clear();
  for (const fn of [clearRepoOverrides, setOrgFilters, setRepoBranches, setRepoIgnoredWorkflows, setRepoIncludeTags]) {
    fn.mockReset().mockResolvedValue(undefined);
  }
});

afterEach(cleanup);

function watched(name: string, id: number, extra: Partial<WatchedRepo> = {}): WatchedRepo {
  const base = makeWatchedRepo(name);
  return { ...base, repo: { ...base.repo, id }, ...extra };
}

const widgets = watched("acme/widgets", 7);
const gadgets = watched("acme/gadgets", 8);
const gizmos = watched("acme/gizmos", 9);
const other = watched("globex/tools", 10, { include_tags: true });

function acme(filters: Partial<OrgFilters> = {}, patch: Partial<Organization> = {}): Organization {
  return makeOrganization("acme", { filters: { ...NONE, ...filters }, repo_count: 3, ...patch });
}

function viewOf(
  repos: WatchedRepo[] = [{ ...widgets, include_tags: true }, gadgets, gizmos],
  patch: Partial<SettingsView> = {},
): SettingsView {
  return makeView({
    accounts: [makeSettingsAccount()],
    organizations: [acme(), makeOrganization("globex")],
    repos: [...repos, other],
    ...patch,
  });
}

function renderTab(view = viewOf()) {
  const result = renderWithSettings(<BranchesTab />, { view });
  return { ...result, save: result.context.saveSettings, update: (v: SettingsView) => result.update({ view: v }) };
}

/** Renders the pane with the acme organization selected. */
function renderOrg(view = viewOf()) {
  localStorage.setItem(SCOPE_KEY, "github.com/acme");
  return renderTab(view);
}

const sidebar = () => screen.getByRole("listbox", { name: "Filters" });
const options = () => within(sidebar()).getAllByRole("option");
const option = (name: string) => within(sidebar()).getByRole("option", { name: new RegExp(`^${name}`) });
const textbox = (name: string) => screen.getByRole("textbox", { name }) as HTMLInputElement;
const popup = (name: string) => screen.getByRole("combobox", { name }) as HTMLSelectElement;
const button = (name: string) => screen.getByRole("button", { name }) as HTMLButtonElement;
const exception = (name: string) => button(name);
const exceptionNames = () =>
  Array.from(document.querySelectorAll(".exception__header")).map((el) => el.querySelector(".exception__name")?.textContent);
const boxes = () => within(screen.getByRole("dialog")).getAllByRole("checkbox");
const choiceNames = () =>
  Array.from(screen.getByRole("dialog").querySelectorAll(".exceptions__choice")).map((el) => el.textContent);

describe("sidebar", () => {
  it("lists All Repositories first, then organizations sorted under their hosts", () => {
    renderTab(
      viewOf([], {
        organizations: [
          makeOrganization("platform", { host: "gitlab.example.com" }),
          makeOrganization("team-10"),
          makeOrganization("team-9"),
          makeOrganization("acme"),
        ],
      }),
    );
    expect(options().map((o) => o.textContent)).toEqual(["All Repositories", "acme", "team-9", "team-10", "platform"]);
    const groups = within(sidebar()).getAllByRole("group");
    expect(groups.map((g) => g.getAttribute("aria-labelledby")).map((id) => document.getElementById(id as string)?.textContent)).toEqual([
      "github.com",
      "gitlab.example.com",
    ]);
    expect(within(groups[1]).getAllByRole("option").map((o) => o.textContent)).toEqual(["platform"]);
  });

  it("marks an organization that sets any filter as Custom", () => {
    renderTab(
      viewOf([], {
        organizations: [
          makeOrganization("a-plain"),
          makeOrganization("b-branches", { filters: { ...NONE, branch_patterns: [] } }),
          makeOrganization("c-ignored", { filters: { ...NONE, ignored_workflows: ["nightly"] } }),
          makeOrganization("d-tags", { filters: { ...NONE, include_tags: false } }),
        ],
      }),
    );
    expect(options().map((o) => o.querySelector(".source-list__badge")?.textContent ?? "")).toEqual([
      "",
      "",
      "Custom",
      "Custom",
      "Custom",
    ]);
  });

  it("starts on All Repositories and selects by click, focusing nothing", () => {
    renderTab();
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { name: "All repositories" })).toBeTruthy();
    fireEvent.click(option("acme"));
    expect(option("acme").getAttribute("aria-selected")).toBe("true");
    expect(option("acme").getAttribute("tabindex")).toBe("0");
    expect(options()[0].getAttribute("tabindex")).toBe("-1");
    expect(screen.getByRole("heading", { name: "acme" })).toBeTruthy();
    expect(localStorage.getItem(SCOPE_KEY)).toBe("github.com/acme");
  });

  it("moves the selection with the arrow keys and focuses the selected option", () => {
    renderTab();
    fireEvent.keyDown(sidebar(), { key: "ArrowDown" });
    expect(option("acme").getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(option("acme"));
    fireEvent.keyDown(sidebar(), { key: "End" });
    expect(option("globex").getAttribute("aria-selected")).toBe("true");
    fireEvent.keyDown(sidebar(), { key: "Home" });
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
  });

  it("remembers the selection and falls back to All Repositories when the organization is gone", () => {
    const first = renderTab();
    fireEvent.click(option("globex"));
    first.unmount();
    renderTab();
    expect(option("globex").getAttribute("aria-selected")).toBe("true");
    cleanup();
    localStorage.setItem(SCOPE_KEY, "github.com/gone");
    renderTab();
    expect(options()[0].getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("heading", { name: "All repositories" })).toBeTruthy();
  });
});

describe("global filters", () => {
  it("saves the branch list and the ignored workflows through the settings saver", () => {
    const { save } = renderTab();
    fireEvent.change(textbox("Branches"), { target: { value: "main, release/*" } });
    fireEvent.blur(textbox("Branches"));
    fireEvent.change(textbox("Ignored workflows"), { target: { value: "Dependabot*, nightly" } });
    fireEvent.blur(textbox("Ignored workflows"));
    expect(save).toHaveBeenCalledWith({ branch_patterns: ["main", "release/*"] });
    expect(save).toHaveBeenCalledWith({ ignored_workflows: ["Dependabot*", "nightly"] });
  });

  it("describes each global control with the text under its label and explains patterns once", () => {
    renderTab();
    for (const [control, name] of [
      [textbox("Branches"), "global-branches"],
      [textbox("Ignored workflows"), "global-ignored-workflows"],
      [screen.getByRole("checkbox", { name: "Include tag runs" }), "global-include-tags"],
    ] as const) {
      const id = control.getAttribute("aria-describedby");
      expect(id).toBe(`${name}-description`);
      expect(document.getElementById(id as string)?.textContent).not.toBe("");
    }
    expect(screen.getByText("Separate patterns with commas, such as main, release/*. Matching ignores case.")).toBeTruthy();
  });

  it("treats missing global lists as empty", () => {
    renderTab(viewOf(undefined, { settings: { branch_patterns: [] } as never }));
    expect(textbox("Ignored workflows").value).toBe("");
    expect((screen.getByRole("checkbox", { name: "Include tag runs" }) as HTMLInputElement).checked).toBe(false);
  });

  it("saves the tag toggle", () => {
    const { save } = renderTab();
    fireEvent.click(screen.getByRole("checkbox", { name: "Include tag runs" }));
    expect(save).toHaveBeenCalledWith({ include_tags: true });
  });

  it("tidies the typed text without saving when the list is unchanged", () => {
    const { save } = renderTab();
    fireEvent.change(textbox("Branches"), { target: { value: " , " } });
    fireEvent.blur(textbox("Branches"));
    expect(textbox("Branches").value).toBe("");
    expect(save).not.toHaveBeenCalled();
  });

  it("commits on Return, ignores other keys and does nothing on a blur without typing", () => {
    const { save } = renderTab();
    const field = textbox("Branches");
    fireEvent.blur(field);
    field.focus();
    fireEvent.change(field, { target: { value: "main" } });
    fireEvent.keyDown(field, { key: "a" });
    expect(save).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: "Enter" });
    expect(save).toHaveBeenCalledWith({ branch_patterns: ["main"] });
  });

  it("follows a value changed elsewhere unless text is being typed", () => {
    const { update } = renderTab();
    update(viewOf(undefined, { settings: makeSettings({ branch_patterns: ["main"] }) }));
    expect(textbox("Branches").value).toBe("main");
    fireEvent.change(textbox("Branches"), { target: { value: "develop" } });
    update(viewOf(undefined, { settings: makeSettings({ branch_patterns: ["trunk"] }) }));
    expect(textbox("Branches").value).toBe("develop");
  });

  it("shows the committed text until the saved value changes", () => {
    const { update } = renderTab();
    fireEvent.change(textbox("Branches"), { target: { value: "main,  release/*" } });
    fireEvent.blur(textbox("Branches"));
    expect(textbox("Branches").value).toBe("main, release/*");
    update(viewOf(undefined, { settings: makeSettings({ branch_patterns: ["release/*"] }) }));
    expect(textbox("Branches").value).toBe("release/*");
  });

  it("saves a half-typed list when the pane closes, and nothing when it is unchanged", () => {
    const first = renderTab();
    fireEvent.change(textbox("Branches"), { target: { value: "develop" } });
    first.unmount();
    expect(first.save).toHaveBeenCalledWith({ branch_patterns: ["develop"] });
    const second = renderTab();
    fireEvent.change(textbox("Branches"), { target: { value: " " } });
    second.unmount();
    expect(second.save).not.toHaveBeenCalled();
  });

  it("disables every field when the config is read-only", () => {
    renderTab(viewOf(undefined, { read_only: true }));
    for (const el of document.querySelectorAll(".accounts__detail input, .accounts__detail select")) {
      expect((el as HTMLInputElement).disabled).toBe(true);
    }
  });

  it("lists at most three known limits", () => {
    renderTab();
    const limits = screen.getByRole("heading", { name: "Known limits" }).closest("section") as HTMLElement;
    expect(within(limits).getAllByRole("listitem").length).toBeLessThanOrEqual(3);
  });
});

describe("organization filters", () => {
  it("names the organization, its host and how many repositories are watched", () => {
    const { update } = renderOrg();
    expect(screen.getByRole("heading", { name: "acme" })).toBeTruthy();
    expect(screen.getByText("github.com · 3 repositories watched")).toBeTruthy();
    expect(screen.getByText("Default uses the settings for all repositories.")).toBeTruthy();
    update(viewOf(undefined, { organizations: [acme({}, { repo_count: 1 })] }));
    expect(screen.getByText("github.com · 1 repository watched")).toBeTruthy();
  });

  it("disables Default fields and shows the global values as their placeholders", () => {
    renderOrg(
      viewOf(undefined, {
        settings: makeSettings({ branch_patterns: ["main", "release/*"], ignored_workflows: ["Dependabot*"], include_tags: true }),
      }),
    );
    expect(popup("Branches filter mode for acme").value).toBe("default");
    expect(textbox("Branches for acme").disabled).toBe(true);
    expect(textbox("Branches for acme").placeholder).toBe("main, release/*");
    expect(textbox("Ignored workflows for acme").disabled).toBe(true);
    expect(textbox("Ignored workflows for acme").placeholder).toBe("Dependabot*");
    expect(within(popup("Tag runs for acme")).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "Default (On)",
      "On",
      "Off",
    ]);
  });

  it("saves an empty list when switching to Custom, with the other filters kept", () => {
    renderOrg(viewOf(undefined, { organizations: [acme({ include_tags: true })] }));
    fireEvent.change(popup("Branches filter mode for acme"), { target: { value: "custom" } });
    expect(setOrgFilters).toHaveBeenLastCalledWith("github.com", "acme", { ...NONE, branch_patterns: [], include_tags: true });
    fireEvent.change(popup("Ignored workflows filter mode for acme"), { target: { value: "custom" } });
    expect(setOrgFilters).toHaveBeenLastCalledWith("github.com", "acme", { ...NONE, ignored_workflows: [], include_tags: true });
  });

  it("enables an empty Custom field with a placeholder that says what empty means", () => {
    renderOrg(
      viewOf(undefined, {
        organizations: [acme({ branch_patterns: [], ignored_workflows: [] })],
        settings: makeSettings({ branch_patterns: ["main"], ignored_workflows: ["Dependabot*"] }),
      }),
    );
    expect(textbox("Branches for acme").disabled).toBe(false);
    expect(textbox("Branches for acme").placeholder).toBe("Default branch");
    expect(textbox("Ignored workflows for acme").disabled).toBe(false);
    expect(textbox("Ignored workflows for acme").placeholder).toBe("None");
  });

  it("saves typed patterns and an emptied field as an explicit empty list", () => {
    renderOrg(viewOf(undefined, { organizations: [acme({ branch_patterns: [], ignored_workflows: ["nightly"] })] }));
    fireEvent.change(textbox("Branches for acme"), { target: { value: "main, release/*" } });
    fireEvent.blur(textbox("Branches for acme"));
    expect(setOrgFilters).toHaveBeenLastCalledWith("github.com", "acme", {
      ...NONE,
      branch_patterns: ["main", "release/*"],
      ignored_workflows: ["nightly"],
    });
    fireEvent.change(textbox("Ignored workflows for acme"), { target: { value: "" } });
    fireEvent.blur(textbox("Ignored workflows for acme"));
    expect(setOrgFilters).toHaveBeenLastCalledWith("github.com", "acme", { ...NONE, branch_patterns: [], ignored_workflows: [] });
  });

  it("saves null when switching back to Default", () => {
    renderOrg(viewOf(undefined, { organizations: [acme({ ignored_workflows: ["nightly"] })] }));
    fireEvent.change(popup("Ignored workflows filter mode for acme"), { target: { value: "default" } });
    expect(setOrgFilters).toHaveBeenCalledWith("github.com", "acme", NONE);
  });

  it("maps the Tag runs choices to null, true and false", () => {
    renderOrg();
    const tags = popup("Tag runs for acme");
    expect(within(tags).getAllByRole("option").map((o) => o.textContent)).toEqual(["Default (Off)", "On", "Off"]);
    for (const value of ["on", "off", "inherit"]) {
      fireEvent.change(tags, { target: { value } });
    }
    expect(setOrgFilters.mock.calls.map((c) => (c[2] as OrgFilters).include_tags)).toEqual([true, false, null]);
  });

  it("reloads and clears the error after a save", async () => {
    const { context } = renderOrg();
    fireEvent.change(popup("Tag runs for acme"), { target: { value: "on" } });
    await waitFor(() => expect(context.reload).toHaveBeenCalled());
    expect(context.setError).toHaveBeenCalledWith(null);
  });

  it("reports a failed save", async () => {
    setOrgFilters.mockRejectedValue("unknown account");
    const { context } = renderOrg();
    fireEvent.change(popup("Tag runs for acme"), { target: { value: "on" } });
    await waitFor(() => expect(context.setError).toHaveBeenCalledWith("That account no longer exists. Reopen Settings and try again."));
    expect(context.reload).not.toHaveBeenCalled();
  });

  it("disables every control when the config is read-only, leaving exceptions expandable", () => {
    renderOrg(viewOf(undefined, { organizations: [acme({ branch_patterns: [] })], read_only: true }));
    fireEvent.click(exception("widgets"));
    for (const el of document.querySelectorAll(".accounts__detail input, .accounts__detail select")) {
      expect((el as HTMLInputElement).disabled).toBe(true);
    }
    expect(button("Add Exceptions…").disabled).toBe(true);
    expect(button("Remove Exception").disabled).toBe(true);
    expect(exception("widgets").getAttribute("aria-expanded")).toBe("true");
  });

  it("shows a GitLab organization with its host and the folder symbol without a known account", () => {
    renderTab(
      viewOf([watched("platform/gateway", 20, { host: "gitlab.example.com", owner: "platform", account_id: "gone" })], {
        organizations: [makeOrganization("platform", { host: "gitlab.example.com" })],
      }),
    );
    fireEvent.click(option("platform"));
    expect(screen.getByText("gitlab.example.com · 1 repository watched")).toBeTruthy();
    expect(document.querySelector(".detail-header .symbol--folder")).not.toBeNull();
  });
});

describe("repository exceptions", () => {
  it("says every repository uses the settings above when none overrides", () => {
    renderOrg(viewOf([widgets, gadgets]));
    expect(screen.getByText("No exceptions. Every repository in acme uses the settings above.")).toBeTruthy();
    expect(button("Add Exceptions…").disabled).toBe(false);
    expect(button("Remove Exception").disabled).toBe(true);
  });

  it("lists only this organization's repositories with an override, by name, with what they set", () => {
    renderOrg(
      viewOf([
        { ...widgets, branch_patterns: ["release/*"] },
        { ...gadgets, ignored_workflows: [], include_tags: true },
        gizmos,
      ]),
    );
    expect(exceptionNames()).toEqual(["widgets", "gadgets"]);
    const description = exception("gadgets").getAttribute("aria-describedby");
    expect(document.getElementById(description as string)?.textContent).toBe("Ignored workflows: None · Tag runs: On");
    expect(screen.queryByRole("button", { name: "tools" })).toBeNull();
    expect(screen.queryByRole("button", { name: "gizmos" })).toBeNull();
  });

  it("expands one exception at a time by click and collapses on a second click", () => {
    renderOrg(viewOf([{ ...widgets, include_tags: true }, { ...gadgets, include_tags: false }]));
    expect(exception("widgets").getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("combobox", { name: "Tag runs for acme/widgets" })).toBeNull();
    fireEvent.click(exception("widgets"));
    expect(exception("widgets").getAttribute("aria-expanded")).toBe("true");
    const body = screen.getByRole("group", { name: "Filters for acme/widgets" });
    expect(exception("widgets").getAttribute("aria-controls")).toBe(body.id);
    expect(popup("Tag runs for acme/widgets").value).toBe("on");
    fireEvent.click(exception("gadgets"));
    expect(exception("widgets").getAttribute("aria-expanded")).toBe("false");
    expect(popup("Tag runs for acme/gadgets").value).toBe("off");
    fireEvent.click(exception("gadgets"));
    expect(exception("gadgets").getAttribute("aria-expanded")).toBe("false");
    expect(exception("gadgets").hasAttribute("aria-controls")).toBe(false);
  });

  it("expands with →, collapses with ← and moves between exceptions with ↑ and ↓", () => {
    renderOrg(viewOf([{ ...widgets, include_tags: true }, { ...gadgets, include_tags: true }]));
    const first = exception("widgets");
    fireEvent.focus(first);
    fireEvent.keyDown(first, { key: "ArrowRight" });
    expect(first.getAttribute("aria-expanded")).toBe("true");
    fireEvent.keyDown(first, { key: "ArrowLeft" });
    expect(first.getAttribute("aria-expanded")).toBe("false");
    fireEvent.keyDown(first, { key: "ArrowRight" });
    fireEvent.keyDown(first, { key: "ArrowDown" });
    expect(document.activeElement).toBe(exception("gadgets"));
    fireEvent.keyDown(exception("gadgets"), { key: "ArrowLeft" });
    expect(first.getAttribute("aria-expanded")).toBe("true");
    fireEvent.keyDown(exception("gadgets"), { key: "ArrowUp" });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: "a" });
    expect(first.getAttribute("aria-expanded")).toBe("true");
  });

  it("shows the organization's value, or the global one, as an exception's Default", () => {
    renderOrg(
      viewOf([{ ...widgets, include_tags: false }], {
        organizations: [acme({ branch_patterns: ["trunk"], include_tags: true })],
        settings: makeSettings({ branch_patterns: ["main"], ignored_workflows: ["Dependabot*"], include_tags: false }),
      }),
    );
    fireEvent.click(exception("widgets"));
    expect(textbox("Branches for acme/widgets").placeholder).toBe("trunk");
    expect(textbox("Branches for acme/widgets").disabled).toBe(true);
    expect(textbox("Ignored workflows for acme/widgets").placeholder).toBe("Dependabot*");
    expect(within(popup("Tag runs for acme/widgets")).getAllByRole("option")[0].textContent).toBe("Default (On)");
  });

  it("saves each filter of an exception through its repository command", () => {
    renderOrg(viewOf([{ ...widgets, branch_patterns: [], ignored_workflows: ["nightly"] }]));
    fireEvent.click(exception("widgets"));
    fireEvent.change(textbox("Branches for acme/widgets"), { target: { value: "hotfix/*" } });
    fireEvent.blur(textbox("Branches for acme/widgets"));
    expect(setRepoBranches).toHaveBeenLastCalledWith("acc-1", 7, ["hotfix/*"]);
    fireEvent.change(popup("Branches filter mode for acme/widgets"), { target: { value: "default" } });
    expect(setRepoBranches).toHaveBeenLastCalledWith("acc-1", 7, null);
    fireEvent.change(popup("Ignored workflows filter mode for acme/widgets"), { target: { value: "default" } });
    expect(setRepoIgnoredWorkflows).toHaveBeenLastCalledWith("acc-1", 7, null);
    for (const value of ["on", "off", "inherit"]) {
      fireEvent.change(popup("Tag runs for acme/widgets"), { target: { value } });
    }
    expect(setRepoIncludeTags.mock.calls).toEqual([
      ["acc-1", 7, true],
      ["acc-1", 7, false],
      ["acc-1", 7, null],
    ]);
    expect(setOrgFilters).not.toHaveBeenCalled();
  });

  it("saves an empty list when an exception switches to Custom", () => {
    renderOrg();
    fireEvent.click(exception("widgets"));
    fireEvent.change(popup("Ignored workflows filter mode for acme/widgets"), { target: { value: "custom" } });
    expect(setRepoIgnoredWorkflows).toHaveBeenCalledWith("acc-1", 7, []);
  });

  it("reports a failed exception save", async () => {
    setRepoIncludeTags.mockRejectedValue("unknown account");
    const { context } = renderOrg();
    fireEvent.click(exception("widgets"));
    fireEvent.change(popup("Tag runs for acme/widgets"), { target: { value: "off" } });
    await waitFor(() => expect(context.setError).toHaveBeenCalledWith("That account no longer exists. Reopen Settings and try again."));
    expect(context.reload).not.toHaveBeenCalled();
  });

  it("removes the selected exception with one call and selects the next one", async () => {
    const { context } = renderOrg(viewOf([{ ...widgets, include_tags: true }, { ...gadgets, include_tags: true }]));
    expect(button("Remove Exception").disabled).toBe(true);
    fireEvent.click(exception("widgets"));
    expect(button("Remove Exception").disabled).toBe(false);
    fireEvent.click(button("Remove Exception"));
    await waitFor(() => expect(context.reload).toHaveBeenCalled());
    expect(clearRepoOverrides).toHaveBeenCalledTimes(1);
    expect(clearRepoOverrides).toHaveBeenCalledWith("acc-1", 7);
    await waitFor(() => expect(exception("gadgets").closest(".exception")?.classList).toContain("exception--selected"));
    expect(exception("widgets").getAttribute("aria-expanded")).toBe("false");
  });

  it("selects the previous exception after removing the last one, and none after the only one", async () => {
    const { update } = renderOrg(viewOf([{ ...widgets, include_tags: true }, { ...gadgets, include_tags: true }]));
    fireEvent.focus(exception("gadgets"));
    fireEvent.click(button("Remove Exception"));
    await waitFor(() => expect(exception("widgets").closest(".exception")?.classList).toContain("exception--selected"));
    expect(exception("gadgets").getAttribute("aria-expanded")).toBe("false");
    update(viewOf([{ ...widgets, include_tags: true }]));
    fireEvent.click(button("Remove Exception"));
    await waitFor(() => expect(button("Remove Exception").disabled).toBe(true));
  });

  it("reports a failed removal", async () => {
    clearRepoOverrides.mockRejectedValue("unknown account");
    const { context } = renderOrg();
    fireEvent.click(exception("widgets"));
    fireEvent.click(button("Remove Exception"));
    await waitFor(() => expect(context.setError).toHaveBeenCalledWith("That account no longer exists. Reopen Settings and try again."));
    expect(context.reload).not.toHaveBeenCalled();
  });
});

describe("Add Exceptions sheet", () => {
  it("lists only this organization's repositories without an exception, filtered by the search", () => {
    renderOrg();
    fireEvent.click(button("Add Exceptions…"));
    expect(screen.getByRole("dialog", { name: "Add Exceptions" })).toBeTruthy();
    expect(choiceNames()).toEqual(["gadgets", "gizmos"]);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search repositories" }), { target: { value: "GIZ" } });
    expect(choiceNames()).toEqual(["gizmos"]);
    fireEvent.change(screen.getByRole("searchbox", { name: "Search repositories" }), { target: { value: "acme" } });
    expect(screen.getByText("No matching repositories.")).toBeTruthy();
  });

  it("names each repository's account when the organization spans several accounts", () => {
    renderOrg(
      viewOf([widgets, { ...gadgets, account_id: "acc-2" }, { ...gizmos, account_id: "acc-9" }], {
        accounts: [makeSettingsAccount(), makeSettingsAccount({ id: "acc-2", label: "Example" })],
      }),
    );
    fireEvent.click(button("Add Exceptions…"));
    expect(choiceNames()).toEqual(["widgetsAcme", "gadgetsExample", "gizmos"]);
  });

  it("widens to fit the longest name within the window", () => {
    renderOrg(viewOf([watched("acme/customer-notification-delivery-pipeline-integration-tests", 30)]));
    fireEvent.click(button("Add Exceptions…"));
    const { style } = screen.getByRole("dialog");
    expect(style.width).toBe("max-content");
    expect(style.minWidth).toBe("440px");
    expect(style.maxWidth).toBe("calc(100vw - 48px)");
    expect(choiceNames()).toEqual(["customer-notification-delivery-pipeline-integration-tests"]);
  });

  it("disables Add until a repository is chosen and unchecks on a second click", () => {
    renderOrg();
    fireEvent.click(button("Add Exceptions…"));
    const add = button("Add");
    expect(add.disabled).toBe(true);
    fireEvent.click(boxes()[0]);
    expect(add.disabled).toBe(false);
    fireEvent.click(boxes()[0]);
    expect(add.disabled).toBe(true);
  });

  it("adds the chosen repositories as Default rows and expands and focuses the first", () => {
    renderOrg();
    fireEvent.click(button("Add Exceptions…"));
    const checks = boxes();
    fireEvent.click(checks[1]);
    fireEvent.click(checks[0]);
    fireEvent.click(button("Add"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(exceptionNames()).toEqual(["widgets", "gadgets", "gizmos"]);
    expect(exception("gadgets").getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(exception("gadgets"));
    expect(exception("gadgets").hasAttribute("aria-describedby")).toBe(false);
    expect(popup("Tag runs for acme/gadgets").value).toBe("inherit");
  });

  it("keeps a session-added row after a refresh and a switch of organization, and drops it on removal", async () => {
    const { update } = renderOrg();
    fireEvent.click(button("Add Exceptions…"));
    fireEvent.click(boxes()[0]);
    fireEvent.click(button("Add"));
    update(viewOf());
    fireEvent.click(option("globex"));
    expect(exceptionNames()).toEqual(["tools"]);
    fireEvent.click(option("acme"));
    expect(exceptionNames()).toEqual(["widgets", "gadgets"]);
    fireEvent.click(exception("gadgets"));
    fireEvent.click(button("Remove Exception"));
    await waitFor(() => expect(clearRepoOverrides).toHaveBeenCalledWith("acc-1", 8));
    update(viewOf());
    await waitFor(() => expect(exceptionNames()).toEqual(["widgets"]));
  });

  it("closes on Cancel and disables the plus button when every repository has a row", () => {
    const { update } = renderOrg();
    fireEvent.click(button("Add Exceptions…"));
    fireEvent.click(button("Cancel"));
    expect(screen.queryByRole("dialog")).toBeNull();
    update(viewOf([{ ...widgets, include_tags: true }, { ...gadgets, include_tags: true }]));
    expect(button("Add Exceptions…").disabled).toBe(true);
  });
});

describe("empty pane", () => {
  it("asks to add an account first, disabled when locked or the Keychain is blocked", () => {
    const { context, update } = renderTab(makeView());
    fireEvent.click(button("Add Account…"));
    expect(context.requestAdd).toHaveBeenCalled();
    update(makeView({ read_only: true }));
    expect(button("Add Account…").disabled).toBe(true);
    update(makeView({ secrets_blocked: true }));
    expect(button("Add Account…").disabled).toBe(true);
  });
});
