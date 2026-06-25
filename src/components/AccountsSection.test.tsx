import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { screen, within } from "@testing-library/react";

import { addAccount, getTokenHealth, removeAccount, updateAccountToken } from "../api";
import type { Account } from "../types";
import { renderWithI18n, user } from "../test/utils";
import AccountsSection from "./AccountsSection";

vi.mock("../api", () => ({
  addAccount: vi.fn(),
  getTokenHealth: vi.fn(),
  removeAccount: vi.fn(),
  updateAccountToken: vi.fn(),
}));

const account: Account = {
  id: "acc-1",
  label: "Work",
  provider: "gitlab",
  base_url: "https://gitlab.com",
  identity: { username: "dev", name: null, email: null },
};

beforeEach(() => {
  vi.mocked(addAccount).mockResolvedValue(account.identity);
  vi.mocked(getTokenHealth).mockResolvedValue([]);
  vi.mocked(removeAccount).mockResolvedValue(undefined);
  vi.mocked(updateAccountToken).mockResolvedValue(account.identity);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("AccountsSection", () => {
  it("hides the instance URL for a SaaS provider and shows it for self-managed", async () => {
    renderWithI18n(<AccountsSection accounts={[]} onAccountsChanged={vi.fn()} />);
    expect(screen.queryByText("accounts.instanceUrl")).toBeNull();

    await user().selectOptions(
      screen.getByRole("combobox", { name: "accounts.providerLabel" }),
      "gitlab_self_managed",
    );

    expect(screen.getByText("accounts.instanceUrl")).toBeInTheDocument();
  });

  it("submits a SaaS account with the fixed base URL and trimmed values", async () => {
    const onChanged = vi.fn();
    renderWithI18n(<AccountsSection accounts={[]} onAccountsChanged={onChanged} />);
    const u = user();

    await u.type(screen.getByLabelText("accounts.label"), "  My Lab  ");
    await u.type(screen.getByPlaceholderText("glpat-..."), "  tok  ");
    await u.click(screen.getByRole("button", { name: "accounts.connect" }));

    expect(addAccount).toHaveBeenCalledWith("gitlab", "My Lab", "https://gitlab.com", "tok");
    expect(onChanged).toHaveBeenCalled();
  });

  it("submits a self-managed account with the entered instance URL", async () => {
    renderWithI18n(<AccountsSection accounts={[]} onAccountsChanged={vi.fn()} />);
    const u = user();

    await u.selectOptions(
      screen.getByRole("combobox", { name: "accounts.providerLabel" }),
      "gitlab_self_managed",
    );
    await u.type(screen.getByLabelText("accounts.instanceUrl"), "https://gl.example.com");
    await u.type(screen.getByPlaceholderText("glpat-..."), "tok");
    await u.click(screen.getByRole("button", { name: "accounts.connect" }));

    expect(addAccount).toHaveBeenCalledWith("gitlab", "", "https://gl.example.com", "tok");
  });

  it("disables submit until a token (and a self-managed URL) is provided", async () => {
    renderWithI18n(<AccountsSection accounts={[]} onAccountsChanged={vi.fn()} />);
    const u = user();
    const button = screen.getByRole("button", { name: "accounts.connect" });

    expect(button).toBeDisabled();

    await u.type(screen.getByPlaceholderText("glpat-..."), "tok");
    expect(button).toBeEnabled();

    await u.selectOptions(
      screen.getByRole("combobox", { name: "accounts.providerLabel" }),
      "gitlab_self_managed",
    );
    expect(button).toBeDisabled();
  });

  it("renders the mapped error message when the backend rejects the account", async () => {
    vi.mocked(addAccount).mockRejectedValueOnce({ kind: "unauthorized", message: "bad token" });
    renderWithI18n(<AccountsSection accounts={[]} onAccountsChanged={vi.fn()} />);
    const u = user();

    await u.type(screen.getByPlaceholderText("glpat-..."), "tok");
    await u.click(screen.getByRole("button", { name: "accounts.connect" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("accounts.error.unauthorized");
  });

  it("removes an account and refreshes the list", async () => {
    const onChanged = vi.fn();
    renderWithI18n(<AccountsSection accounts={[account]} onAccountsChanged={onChanged} />);

    await user().click(screen.getByRole("button", { name: "common.remove" }));

    expect(removeAccount).toHaveBeenCalledWith("acc-1");
    expect(onChanged).toHaveBeenCalled();
  });

  it("flags an account whose token health reports auth failure", async () => {
    vi.mocked(getTokenHealth).mockResolvedValue([
      { account_id: "acc-1", auth_failed: true, expires_at: null },
    ]);
    renderWithI18n(<AccountsSection accounts={[account]} onAccountsChanged={vi.fn()} />);

    expect(await screen.findByText("accounts.tokenInvalid")).toBeInTheDocument();
  });

  it("shows an expiry warning for a token expiring within the warning window", async () => {
    const soon = new Date(Date.now() + 2 * 86_400_000).toISOString().slice(0, 10);
    vi.mocked(getTokenHealth).mockResolvedValue([
      { account_id: "acc-1", auth_failed: false, expires_at: soon },
    ]);
    renderWithI18n(<AccountsSection accounts={[account]} onAccountsChanged={vi.fn()} />);

    expect(await screen.findByText("accounts.expiresInDays")).toBeInTheDocument();
  });

  it("updates an account token in place via the inline editor", async () => {
    const onChanged = vi.fn();
    renderWithI18n(<AccountsSection accounts={[account]} onAccountsChanged={onChanged} />);
    const u = user();

    await u.click(screen.getByRole("button", { name: "accounts.updateToken" }));
    const row = screen.getByRole("listitem");
    await u.type(within(row).getByLabelText("accounts.token"), "  newtok  ");
    await u.click(within(row).getByRole("button", { name: "accounts.save" }));

    expect(updateAccountToken).toHaveBeenCalledWith("acc-1", "newtok");
    expect(onChanged).toHaveBeenCalled();
  });
});
