// @vitest-environment jsdom
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AccountSnapshot } from "../lib/snapshot";
import type { SettingsView } from "../lib/tauri";
import { makeAccount, makeSettingsAccount, makeSnapshot, makeView as makeBaseView, makeWatchedRepo } from "../test/fixtures";
import { makeSettingsContext, renderWithSettings } from "../test/settings";
import type { SheetRequest } from "./SettingsContext";

const addAccount = vi.fn();
const deleteAccount = vi.fn();
const deleteAllAccounts = vi.fn();
const openGithubTokenPage = vi.fn();
const renameAccount = vi.fn();
const replaceToken = vi.fn();
const testConnection = vi.fn();

vi.mock("../lib/tauri", () => ({
  addAccount: (...a: unknown[]) => addAccount(...a),
  deleteAccount: (...a: unknown[]) => deleteAccount(...a),
  deleteAllAccounts: (...a: unknown[]) => deleteAllAccounts(...a),
  openGithubTokenPage: (...a: unknown[]) => openGithubTokenPage(...a),
  renameAccount: (...a: unknown[]) => renameAccount(...a),
  replaceToken: (...a: unknown[]) => replaceToken(...a),
  testConnection: (...a: unknown[]) => testConnection(...a),
}));

import { AccountsTab } from "./AccountsTab";

const TOKEN = "github_pat_example";

const github = makeSettingsAccount({ id: "a1" });
const gitlab = makeSettingsAccount({ id: "a2", kind: "gitlab", label: "Example GitLab", base_url: "https://git.example.com/", login: "dev" });

function makeView(patch: Partial<SettingsView> = {}): SettingsView {
  return makeBaseView({
    accounts: [github, gitlab],
    repos: [
      makeWatchedRepo("acme/one", { account_id: "a1" }),
      makeWatchedRepo("acme/two-repo", { account_id: "a1" }),
      makeWatchedRepo("example/three", { account_id: "a2" }),
    ],
    ...patch,
  });
}

function status(id: string, patch: Partial<AccountSnapshot> = {}): AccountSnapshot {
  return makeAccount({ id, ...patch });
}

let context = makeSettingsContext();
const onRequestHandled = vi.fn();

function mount(view = makeView(), accounts?: AccountSnapshot[], request: SheetRequest | null = null) {
  const snapshot = accounts ? makeSnapshot({ accounts }) : null;
  const result = renderWithSettings(<AccountsTab request={request} onRequestHandled={onRequestHandled} />, { view, snapshot });
  context = result.context;
  return {
    ...result,
    update(next: SettingsView, nextAccounts?: AccountSnapshot[]) {
      result.update({ view: next, snapshot: nextAccounts ? makeSnapshot({ accounts: nextAccounts }) : null });
    },
    request(next: SheetRequest | null) {
      result.update({}, <AccountsTab request={next} onRequestHandled={onRequestHandled} />);
    },
  };
}

function dialog() {
  return screen.getByRole("dialog");
}

function accountOptions() {
  return within(screen.getByRole("listbox", { name: "Accounts" })).getAllByRole("option");
}

function alert() {
  return screen.getByRole("alertdialog");
}

function moreActions() {
  return screen.getByRole("combobox", { name: "More Actions" }) as HTMLSelectElement;
}

function field(label: string): HTMLInputElement {
  return within(screen.getByRole("dialog")).getByLabelText(label) as HTMLInputElement;
}

function typeInto(label: string, value: string) {
  fireEvent.change(field(label), { target: { value } });
}

beforeEach(() => {
  for (const fn of [addAccount, deleteAccount, deleteAllAccounts, openGithubTokenPage, renameAccount, replaceToken, testConnection, onRequestHandled]) {
    fn.mockReset();
  }
  addAccount.mockResolvedValue({ id: "a3", kind: "github", label: "New" });
  deleteAccount.mockResolvedValue(undefined);
  deleteAllAccounts.mockResolvedValue(2);
  renameAccount.mockResolvedValue(undefined);
  replaceToken.mockResolvedValue(undefined);
  testConnection.mockResolvedValue({ user_id: 1, login: "example-user" });
});

afterEach(cleanup);

describe("empty state", () => {
  it("offers Add Account and opens the sheet", () => {
    mount(makeView({ accounts: [], repos: [] }));
    fireEvent.click(screen.getByRole("button", { name: "Add Account…" }));
    expect(screen.getByRole("dialog", { name: "Add Account" })).toBeTruthy();
  });

  it("disables Add Account when the config is read-only or the Keychain is blocked", () => {
    const { update } = mount(makeView({ accounts: [], read_only: true }));
    expect((screen.getByRole("button", { name: "Add Account…" }) as HTMLButtonElement).disabled).toBe(true);
    update(makeView({ accounts: [], secrets_blocked: true }));
    expect((screen.getByRole("button", { name: "Add Account…" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("does not show the sheet while the config is locked", () => {
    mount(makeView({ accounts: [], read_only: true }), undefined, { sheet: "add", accountId: null });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("account list", () => {
  it("lists accounts with their login or server and shows the first one's details", () => {
    mount();
    const list = screen.getByRole("listbox", { name: "Accounts" });
    const options = within(list).getAllByRole("option");
    expect(options).toHaveLength(2);
    expect(options[0].textContent).toContain("acme-bot");
    expect(options[1].textContent).toContain("dev");
    expect(screen.getByRole("heading", { name: "Acme" })).toBeTruthy();
    expect(screen.getByText(/GitHub · 2 repositories watched/)).toBeTruthy();
  });

  it("falls back to the host when an account has no login", () => {
    mount(
      makeView({
        accounts: [
          makeSettingsAccount({ id: "x1", label: "Plain", login: undefined }),
          makeSettingsAccount({ id: "x2", kind: "gitlab", label: "Valid", base_url: "https://git.example.com/path", login: undefined }),
          makeSettingsAccount({ id: "x3", kind: "gitlab", label: "Broken", base_url: "not a url", login: undefined }),
        ],
        repos: [],
      }),
    );
    const options = accountOptions();
    expect(options[0].textContent).toContain("github.com");
    expect(options[1].textContent).toContain("git.example.com");
    expect(options[2].textContent).toContain("not a url");
    expect(screen.getByText(/0 repositories watched/)).toBeTruthy();
    expect(screen.getByText("—")).toBeTruthy();
  });

  it("selects an account on click and shows its server and singular repository count", () => {
    mount();
    fireEvent.click(accountOptions()[1]);
    expect(screen.getByRole("heading", { name: "Example GitLab" })).toBeTruthy();
    expect(screen.getByText(/GitLab · 1 repository watched/)).toBeTruthy();
    expect(screen.getByText("git.example.com")).toBeTruthy();
    expect(screen.getByText(/read_api scope/)).toBeTruthy();
  });

  it("keeps one option in the tab order and moves with the arrow, Home and End keys", () => {
    mount(
      makeView({
        accounts: [github, gitlab, makeSettingsAccount({ id: "a3", label: "Third" })],
      }),
    );
    const tabIndexes = () => accountOptions().map((o) => o.getAttribute("tabindex"));
    expect(tabIndexes()).toEqual(["0", "-1", "-1"]);
    fireEvent.keyDown(accountOptions()[0], { key: "ArrowDown" });
    expect(tabIndexes()).toEqual(["-1", "0", "-1"]);
    expect(document.activeElement).toBe(accountOptions()[1]);
    fireEvent.keyDown(accountOptions()[1], { key: "End" });
    expect(tabIndexes()).toEqual(["-1", "-1", "0"]);
    fireEvent.keyDown(accountOptions()[2], { key: "ArrowDown" });
    expect(tabIndexes()).toEqual(["-1", "-1", "0"]);
    fireEvent.keyDown(accountOptions()[2], { key: "ArrowUp" });
    expect(tabIndexes()).toEqual(["-1", "0", "-1"]);
    fireEvent.keyDown(accountOptions()[1], { key: "Home" });
    expect(tabIndexes()).toEqual(["0", "-1", "-1"]);
    fireEvent.keyDown(accountOptions()[0], { key: "ArrowUp" });
    expect(tabIndexes()).toEqual(["0", "-1", "-1"]);
    fireEvent.keyDown(accountOptions()[0], { key: "x" });
    expect(tabIndexes()).toEqual(["0", "-1", "-1"]);
  });

  it("selects the first account again when the selected one disappears", () => {
    const { update } = mount();
    fireEvent.click(accountOptions()[1]);
    update(makeView({ accounts: [github] }));
    expect(screen.getByRole("heading", { name: "Acme" })).toBeTruthy();
  });

  it("disables removal controls when the config is read-only", () => {
    mount(makeView({ read_only: true }));
    expect((screen.getByLabelText("Add account") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByLabelText("Remove account") as HTMLButtonElement).disabled).toBe(true);
    expect(moreActions().disabled).toBe(true);
    expect((screen.getByLabelText("Name") as HTMLInputElement).disabled).toBe(true);
  });

  it("offers Remove All Accounts only with two or more accounts", () => {
    const { update } = mount();
    expect(within(moreActions()).getByRole("option", { name: "Remove All Accounts…" })).toBeTruthy();
    expect(moreActions().disabled).toBe(false);
    update(makeView({ accounts: [github] }));
    expect(moreActions().disabled).toBe(true);
  });

  it("disables Replace Token while the Keychain is blocked", () => {
    mount(makeView({ secrets_blocked: true }));
    expect((screen.getByRole("button", { name: "Replace Token…" }) as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("connection status", () => {
  const cases: { name: string; snapshot: Partial<AccountSnapshot> | null; color: string; text: string; banner: string | null }[] = [
    { name: "unchecked", snapshot: null, color: "gray", text: "Checking…", banner: null },
    { name: "connected", snapshot: {}, color: "green", text: "Connected", banner: null },
    { name: "token rejected", snapshot: { auth_error: true }, color: "red", text: "Token rejected", banner: "The token was rejected" },
    {
      name: "unable to read the Keychain",
      snapshot: { auth_error: true, keychain_denied: true },
      color: "red",
      text: "Can’t read Keychain",
      banner: "The token was rejected",
    },
    { name: "unreachable", snapshot: { unreachable: true }, color: "orange", text: "Unreachable", banner: "The server is unreachable" },
    { name: "rate limited", snapshot: { rate_limited_until: 100 }, color: "orange", text: "Rate limited", banner: null },
  ];

  for (const c of cases) {
    it(`describes an account that is ${c.name} the same way in the list and the Status row`, () => {
      mount(makeView({ accounts: [github], repos: [] }), c.snapshot ? [status("a1", c.snapshot)] : []);
      const option = accountOptions()[0];
      expect(within(option).getByText(c.text)).toBeTruthy();
      expect(within(option).getByTitle(c.text).querySelector(`.dot--${c.color}`)).toBeTruthy();
      expect(screen.getAllByText(c.text)).toHaveLength(2);
      const banner = c.banner ? screen.queryByText(new RegExp(c.banner)) : null;
      expect(Boolean(banner)).toBe(Boolean(c.banner));
    });
  }
});

describe("renaming", () => {
  function nameField() {
    return screen.getByLabelText("Name") as HTMLInputElement;
  }

  it("saves a new name when the field loses focus", async () => {
    mount();
    fireEvent.change(nameField(), { target: { value: "  Acme Corp  " } });
    fireEvent.blur(nameField());
    await waitFor(() => expect(renameAccount).toHaveBeenCalledWith("a1", "Acme Corp"));
    expect(context.reload).toHaveBeenCalled();
    expect(context.setError).toHaveBeenCalledWith(null);
  });

  it("commits on Enter", async () => {
    mount();
    nameField().focus();
    fireEvent.change(nameField(), { target: { value: "Renamed" } });
    fireEvent.keyDown(nameField(), { key: "Enter" });
    await waitFor(() => expect(renameAccount).toHaveBeenCalledWith("a1", "Renamed"));
    fireEvent.keyDown(nameField(), { key: "a" });
  });

  it("restores the old name for an empty value and does not save unchanged names", async () => {
    mount();
    fireEvent.change(nameField(), { target: { value: "   " } });
    fireEvent.blur(nameField());
    expect(nameField().value).toBe("Acme");
    fireEvent.blur(nameField());
    await act(async () => undefined);
    expect(renameAccount).not.toHaveBeenCalled();
  });

  it("restores the old name and reports the error when saving fails", async () => {
    renameAccount.mockRejectedValue("unknown account");
    mount();
    fireEvent.change(nameField(), { target: { value: "Other" } });
    fireEvent.blur(nameField());
    await waitFor(() => expect(nameField().value).toBe("Acme"));
    expect(context.setError).toHaveBeenCalledWith("That account no longer exists. Reopen Settings and try again.");
  });

  it("saves a half-typed name when another account is selected", () => {
    mount();
    fireEvent.change(nameField(), { target: { value: "Half typed" } });
    fireEvent.click(accountOptions()[1]);
    expect(renameAccount).toHaveBeenCalledWith("a1", "Half typed");
  });

  it("does not save on unmount when nothing was typed or the text was cleared", () => {
    const { unmount } = mount();
    fireEvent.click(accountOptions()[1]);
    fireEvent.change(nameField(), { target: { value: "" } });
    unmount();
    expect(renameAccount).not.toHaveBeenCalled();
  });
});

describe("Add Account sheet", () => {
  function open(view = makeView()) {
    mount(view);
    fireEvent.click(screen.getByLabelText("Add account"));
    return dialog();
  }

  function submitButton() {
    return within(dialog()).getByRole("button", { name: /^(Add Account|Adding…)$/ }) as HTMLButtonElement;
  }

  it("keeps Add disabled until a fine-grained token is entered", () => {
    open();
    expect(submitButton().disabled).toBe(true);
    typeInto("Token", TOKEN);
    expect(submitButton().disabled).toBe(false);
  });

  it("flags classic and unknown token types", () => {
    open();
    typeInto("Token", "ghp_example");
    expect(screen.getByText(/classic token/)).toBeTruthy();
    expect(submitButton().disabled).toBe(true);
    typeInto("Token", "something");
    expect(screen.getByText(/not a fine-grained token/)).toBeTruthy();
  });

  it("opens the GitHub token page, with the organization when one is entered", () => {
    open();
    const create = screen.getByRole("button", { name: /Create Token on GitHub/ });
    fireEvent.click(create);
    expect(openGithubTokenPage).toHaveBeenLastCalledWith(null);
    typeInto("Organization", " acme-org ");
    fireEvent.click(create);
    expect(openGithubTokenPage).toHaveBeenLastCalledWith("acme-org");
  });

  it("adds a GitHub account named after the organization and closes the sheet", async () => {
    open();
    typeInto("Organization", "acme-org");
    typeInto("Token", ` ${TOKEN} `);
    fireEvent.click(submitButton());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(addAccount).toHaveBeenCalledWith({
      kind: "github",
      label: "acme-org",
      base_url: undefined,
      token: TOKEN,
      allow_insecure: false,
    });
    expect(context.reload).toHaveBeenCalled();
  });

  it("uses an explicit name over the default", async () => {
    open();
    typeInto("Token", TOKEN);
    typeInto("Name", " My Label ");
    fireEvent.click(submitButton());
    await waitFor(() => expect(addAccount).toHaveBeenCalled());
    expect(addAccount.mock.calls[0][0].label).toBe("My Label");
  });

  it("names a GitHub account after the provider when no organization is given", async () => {
    open();
    typeInto("Token", TOKEN);
    expect(field("Name").placeholder).toBe("GitHub");
  });

  it("prefills the GitLab server and names the account after its host", async () => {
    open();
    fireEvent.click(screen.getByRole("radio", { name: /GitLab/ }));
    expect(field("Server").value).toBe("https://gitlab.com");
    typeInto("Server", " https://git.example.com ");
    typeInto("Token", "glpat-example");
    expect(field("Name").placeholder).toBe("git.example.com");
    fireEvent.click(submitButton());
    await waitFor(() => expect(addAccount).toHaveBeenCalled());
    expect(addAccount).toHaveBeenCalledWith({
      kind: "gitlab",
      label: "git.example.com",
      base_url: "https://git.example.com",
      token: "glpat-example",
      allow_insecure: false,
    });
  });

  it("falls back to GitLab as the name for an address that is not a URL", () => {
    open();
    fireEvent.click(screen.getByRole("radio", { name: /GitLab/ }));
    typeInto("Server", "nonsense");
    expect(field("Name").placeholder).toBe("GitLab");
  });

  it("requires a server address for GitLab", () => {
    open();
    fireEvent.click(screen.getByRole("radio", { name: /GitLab/ }));
    typeInto("Token", "glpat-example");
    expect(submitButton().disabled).toBe(false);
    typeInto("Server", "  ");
    expect(submitButton().disabled).toBe(true);
  });

  it("asks for consent before sending a token over http://", async () => {
    open();
    fireEvent.click(screen.getByRole("radio", { name: /GitLab/ }));
    typeInto("Server", "HTTP://git.example.com");
    typeInto("Token", "glpat-example");
    expect(submitButton().disabled).toBe(true);
    fireEvent.click(screen.getByRole("checkbox"));
    expect(submitButton().disabled).toBe(false);
    fireEvent.click(submitButton());
    await waitFor(() => expect(addAccount).toHaveBeenCalled());
    expect(addAccount.mock.calls[0][0].allow_insecure).toBe(true);
  });

  it("shows the reason and keeps the sheet open when adding fails", async () => {
    addAccount.mockRejectedValue("token rejected");
    open();
    typeInto("Token", TOKEN);
    fireEvent.click(submitButton());
    await waitFor(() => expect(screen.getByText(/server rejected this token/)).toBeTruthy());
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(submitButton().disabled).toBe(false);
  });

  it("clears a shown error when the provider changes", async () => {
    addAccount.mockRejectedValue("token rejected");
    open();
    typeInto("Token", TOKEN);
    fireEvent.click(submitButton());
    await waitFor(() => expect(screen.getByText(/server rejected this token/)).toBeTruthy());
    fireEvent.click(screen.getByRole("radio", { name: /GitLab/ }));
    expect(screen.queryByText(/server rejected this token/)).toBeNull();
  });

  it("closes on Cancel", () => {
    open();
    fireEvent.click(within(dialog()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("is not available while the Keychain is blocked", () => {
    mount(makeView({ secrets_blocked: true }), undefined, { sheet: "add", accountId: null });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  describe("Test Connection", () => {
    it("is disabled until the form is ready", () => {
      open();
      expect((screen.getByRole("button", { name: "Test Connection" }) as HTMLButtonElement).disabled).toBe(true);
      typeInto("Token", TOKEN);
      expect((screen.getByRole("button", { name: "Test Connection" }) as HTMLButtonElement).disabled).toBe(false);
    });

    it("shows a green Connected result with the login for assistive technology", async () => {
      open();
      typeInto("Token", TOKEN);
      fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
      await waitFor(() => expect(within(dialog()).getByText("Connected")).toBeTruthy());
      expect(within(dialog()).getByText(/as example-user/)).toBeTruthy();
      expect(dialog().querySelector(".dot--green")).toBeTruthy();
      expect(dialog().querySelector(".value--status")?.getAttribute("title")).toBe("Connected as example-user");
      expect(testConnection).toHaveBeenCalledWith(expect.objectContaining({ kind: "github", token: TOKEN }));
    });

    it("shows Testing… while the request runs", async () => {
      let finish: (v: unknown) => void = () => undefined;
      testConnection.mockImplementation(() => new Promise((r) => (finish = r)));
      open();
      typeInto("Token", TOKEN);
      fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
      expect(screen.getByRole("button", { name: "Testing…" })).toBeTruthy();
      await act(async () => finish({ user_id: 1, login: "x" }));
      expect(screen.getByRole("button", { name: "Test Connection" })).toBeTruthy();
    });

    it("shows a red Failed result and the reason under the fields, cleared when input changes", async () => {
      testConnection.mockRejectedValue("network error: down");
      open();
      typeInto("Token", TOKEN);
      fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
      await waitFor(() => expect(screen.getByText("Failed")).toBeTruthy());
      expect(dialog().querySelector(".dot--red")).toBeTruthy();
      expect(screen.getByText(/Can't reach the server/)).toBeTruthy();
      typeInto("Token", `${TOKEN}2`);
      expect(screen.queryByText("Failed")).toBeNull();
      expect(screen.queryByText(/Can't reach the server/)).toBeNull();
    });

    it("clears a previous result when tested again", async () => {
      testConnection.mockResolvedValueOnce({ user_id: 1, login: "x" }).mockRejectedValueOnce("rate limited");
      open();
      typeInto("Token", TOKEN);
      fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
      await waitFor(() => expect(within(dialog()).getByText("Connected")).toBeTruthy());
      fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
      await waitFor(() => expect(screen.getByText("Failed")).toBeTruthy());
      expect(screen.queryByText("Connected")).toBeNull();
    });
  });
});

describe("Replace Token sheet", () => {
  function open(index = 0) {
    mount();
    fireEvent.click(accountOptions()[index]);
    fireEvent.click(screen.getByRole("button", { name: "Replace Token…" }));
  }

  function replaceButton() {
    return within(dialog()).getByRole("button", { name: /^(Replace|Replacing…)$/ }) as HTMLButtonElement;
  }

  it("replaces the token and closes", async () => {
    open();
    expect(replaceButton().disabled).toBe(true);
    typeInto("New token", ` ${TOKEN} `);
    fireEvent.click(replaceButton());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(replaceToken).toHaveBeenCalledWith("a1", TOKEN);
    expect(context.reload).toHaveBeenCalled();
  });

  it("offers the GitHub token page for GitHub accounts only", () => {
    open();
    fireEvent.click(within(dialog()).getByRole("button", { name: "Create Token on GitHub" }));
    expect(openGithubTokenPage).toHaveBeenCalledWith(null);
    cleanup();
    open(1);
    expect(within(dialog()).queryByRole("button", { name: "Create Token on GitHub" })).toBeNull();
  });

  it("blocks a token of the wrong type for GitHub", () => {
    open();
    typeInto("New token", "ghp_example");
    expect(screen.getByText(/classic token/)).toBeTruthy();
    expect(replaceButton().disabled).toBe(true);
  });

  it("shows the reason and stays open when replacing fails", async () => {
    replaceToken.mockRejectedValue("keychain access denied");
    open();
    typeInto("New token", TOKEN);
    fireEvent.click(replaceButton());
    await waitFor(() => expect(screen.getByText(/can't use the Keychain/)).toBeTruthy());
    expect(replaceButton().disabled).toBe(false);
  });

  it("tests the new token against the account's server", async () => {
    open(1);
    typeInto("New token", "glpat-example");
    fireEvent.click(within(dialog()).getByRole("button", { name: "Test Connection" }));
    await waitFor(() => expect(within(dialog()).getByText("Connected")).toBeTruthy());
    expect(testConnection).toHaveBeenCalledWith({
      kind: "gitlab",
      label: "Example GitLab",
      base_url: "https://git.example.com/",
      token: "glpat-example",
      allow_insecure: false,
    });
  });

  it("allows an http:// server that the account already uses", async () => {
    mount(makeView({ accounts: [{ ...gitlab, base_url: " HTTP://git.example.com" }] }));
    fireEvent.click(screen.getByRole("button", { name: "Replace Token…" }));
    typeInto("New token", "glpat-example");
    fireEvent.click(within(dialog()).getByRole("button", { name: "Test Connection" }));
    await waitFor(() => expect(testConnection).toHaveBeenCalled());
    expect(testConnection.mock.calls[0][0].allow_insecure).toBe(true);
  });

  it("fails a test of a token with the wrong type without calling the server", () => {
    open();
    typeInto("New token", "ghp_example");
    fireEvent.click(within(dialog()).getByRole("button", { name: "Test Connection" }));
    expect(testConnection).not.toHaveBeenCalled();
    expect(screen.getByText("Failed")).toBeTruthy();
  });

  it("closes on Escape", () => {
    open();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});

describe("removing accounts", () => {
  it("confirms before removing, with Cancel as the default button", async () => {
    mount();
    fireEvent.click(screen.getByLabelText("Remove account"));
    const sheet = alert();
    expect(within(sheet).getByRole("heading", { name: "Remove “Acme”?" })).toBeTruthy();
    expect(sheet.getAttribute("aria-describedby")).toBe(within(sheet).getByText(/stops watching/).id);
    expect(within(sheet).getByText(/2 repositories/)).toBeTruthy();
    expect(within(sheet).getByText(/stays valid on GitHub/)).toBeTruthy();
    expect(within(sheet).getByRole("button", { name: "Cancel" }).className).toBe("default");
    fireEvent.click(within(sheet).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(deleteAccount).toHaveBeenCalledWith("a1");
    expect(context.reload).toHaveBeenCalled();
  });

  it("names GitLab and one repository for a GitLab account", () => {
    mount();
    fireEvent.click(accountOptions()[1]);
    fireEvent.click(screen.getByLabelText("Remove account"));
    expect(within(alert()).getByText(/1 repository/)).toBeTruthy();
    expect(within(alert()).getByText(/stays valid on GitLab/)).toBeTruthy();
  });

  it("shows a busy label while removing", async () => {
    let finish: () => void = () => undefined;
    deleteAccount.mockImplementation(() => new Promise<void>((r) => (finish = r)));
    mount();
    fireEvent.click(screen.getByLabelText("Remove account"));
    fireEvent.click(within(alert()).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(within(alert()).getByRole("button", { name: "Removing…" })).toBeTruthy());
    expect((within(alert()).getByRole("button", { name: "Cancel" }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => finish());
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("keeps the sheet open and reports the error when removal fails", async () => {
    deleteAccount.mockRejectedValue("unknown account");
    mount();
    fireEvent.click(screen.getByLabelText("Remove account"));
    fireEvent.click(within(alert()).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(context.setError).toHaveBeenCalledWith("That account no longer exists. Reopen Settings and try again."));
    expect(alert()).toBeTruthy();
    expect(within(alert()).getByRole("button", { name: "Remove" })).toBeTruthy();
  });

  it("does not remove on Return and cancels instead", () => {
    mount();
    fireEvent.click(screen.getByLabelText("Remove account"));
    fireEvent.keyDown(alert(), { key: "Enter" });
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(deleteAccount).not.toHaveBeenCalled();
  });

  it("removes every account after confirmation", async () => {
    mount();
    fireEvent.change(moreActions(), { target: { value: "0" } });
    const sheet = alert();
    expect(within(sheet).getByRole("heading", { name: "Remove all 2 accounts?" })).toBeTruthy();
    expect(within(sheet).getByRole("button", { name: "Cancel" }).className).toBe("default");
    fireEvent.click(within(sheet).getByRole("button", { name: "Remove All" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(deleteAllAccounts).toHaveBeenCalled();
  });

  it("cancels Remove All without deleting", () => {
    mount();
    fireEvent.change(moreActions(), { target: { value: "0" } });
    fireEvent.click(within(alert()).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(deleteAllAccounts).not.toHaveBeenCalled();
  });
});

describe("sheet requests", () => {
  it("opens the Add Account sheet once and reports the request handled", () => {
    mount(makeView(), undefined, { sheet: "add", accountId: null });
    expect(screen.getByRole("dialog", { name: "Add Account" })).toBeTruthy();
    expect(onRequestHandled).toHaveBeenCalledTimes(1);
  });

  it("selects the named account and opens Replace Token for it", () => {
    const { request } = mount();
    expect(onRequestHandled).not.toHaveBeenCalled();
    request({ sheet: "replace", accountId: "a2" });
    expect(screen.getByRole("dialog", { name: "Replace Token for “Example GitLab”" })).toBeTruthy();
    expect(accountOptions()[1].getAttribute("aria-selected")).toBe("true");
    expect(onRequestHandled).toHaveBeenCalledTimes(1);
  });

  it("ignores a Replace Token request for an unknown account or while the Keychain is blocked", () => {
    const { request } = mount();
    request({ sheet: "replace", accountId: "missing" });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(onRequestHandled).toHaveBeenCalledTimes(1);
    cleanup();
    const blocked = mount(makeView({ secrets_blocked: true }));
    blocked.request({ sheet: "replace", accountId: "a1" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("selects an account added through the sheet", async () => {
    addAccount.mockResolvedValue(gitlab);
    mount(makeView(), undefined, { sheet: "add", accountId: null });
    typeInto("Token", TOKEN);
    fireEvent.click(within(dialog()).getByRole("button", { name: "Add Account" }));
    await waitFor(() => expect(accountOptions()[1].getAttribute("aria-selected")).toBe("true"));
  });
});
