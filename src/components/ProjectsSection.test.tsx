import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { screen } from "@testing-library/react";

import { getMonitoredProjects, listDiscoveredProjects, setMonitoredProjects } from "../api";
import type { Account, DiscoveredProject } from "../types";
import { renderWithI18n, user } from "../test/utils";
import ProjectsSection from "./ProjectsSection";

vi.mock("../api", () => ({
  getMonitoredProjects: vi.fn(),
  listDiscoveredProjects: vi.fn(),
  setMonitoredProjects: vi.fn(),
}));

const accounts: Account[] = [
  {
    id: "acc-1",
    label: "Work",
    provider: "gitlab",
    base_url: "https://gitlab.com",
    identity: { username: "dev", name: null, email: null },
  },
];

const projects: DiscoveredProject[] = [
  { id: 1, name: "web-app", web_url: "https://gl/acme/frontend/web-app", group: "acme/frontend" },
  { id: 2, name: "design-system", web_url: "https://gl/acme/frontend/ds", group: "acme/frontend" },
  { id: 3, name: "api-gateway", web_url: "https://gl/acme/backend/api", group: "acme/backend" },
];

beforeEach(() => {
  vi.mocked(getMonitoredProjects).mockResolvedValue([]);
  vi.mocked(listDiscoveredProjects).mockResolvedValue(projects);
  vi.mocked(setMonitoredProjects).mockResolvedValue(undefined);
});

afterEach(() => {
  vi.clearAllMocks();
});

describe("ProjectsSection", () => {
  it("renders discovered projects grouped by namespace", async () => {
    renderWithI18n(<ProjectsSection accounts={accounts} />);

    expect(await screen.findByRole("button", { name: /acme\/frontend/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /acme\/backend/ })).toBeInTheDocument();
  });

  it("shows an error with retry, and re-fetches discovery on retry", async () => {
    vi.mocked(listDiscoveredProjects).mockRejectedValueOnce(
      Object.assign(new Error("connection refused"), { kind: "network" }),
    );
    renderWithI18n(<ProjectsSection accounts={accounts} />);

    expect(await screen.findByRole("alert")).toHaveTextContent("projects.loadError");

    await user().click(screen.getByRole("button", { name: "common.retry" }));

    expect(await screen.findByRole("button", { name: /acme\/frontend/ })).toBeInTheDocument();
    expect(listDiscoveredProjects).toHaveBeenCalledTimes(2);
  });

  it("filters the visible projects by the search query", async () => {
    renderWithI18n(<ProjectsSection accounts={accounts} />);
    await screen.findByRole("button", { name: /acme\/frontend/ });

    await user().type(screen.getByRole("searchbox"), "web");

    expect(screen.getByText("web-app")).toBeInTheDocument();
    expect(screen.queryByText("design-system")).toBeNull();
    expect(screen.queryByText("api-gateway")).toBeNull();
  });

  it("persists the monitored set when a project is toggled on", async () => {
    renderWithI18n(<ProjectsSection accounts={accounts} />);
    const u = user();
    await u.click(await screen.findByRole("button", { name: /acme\/frontend/ }));

    await u.click(screen.getByRole("checkbox", { name: "web-app" }));

    expect(setMonitoredProjects).toHaveBeenCalledWith("acc-1", [
      expect.objectContaining({ account_id: "acc-1", project_id: 1, name: "web-app" }),
    ]);
  });

  it("monitors every project in a group via the select-all checkbox", async () => {
    renderWithI18n(<ProjectsSection accounts={accounts} />);
    const u = user();
    await screen.findByRole("button", { name: /acme\/frontend/ });
    // Search by group so only acme/frontend (two projects) is shown, leaving one select-all checkbox.
    await u.type(screen.getByRole("searchbox"), "acme/frontend");

    await u.click(screen.getByRole("checkbox", { name: "projects.selectAllIn" }));

    const calls = vi.mocked(setMonitoredProjects).mock.calls;
    const [, monitored] = calls[calls.length - 1];
    expect(monitored.map((m) => m.project_id).sort((a, b) => a - b)).toEqual([1, 2]);
  });

  it("expands and collapses a group's project list", async () => {
    renderWithI18n(<ProjectsSection accounts={accounts} />);
    const u = user();
    const toggle = await screen.findByRole("button", { name: /acme\/frontend/ });
    expect(screen.queryByRole("checkbox", { name: "web-app" })).toBeNull();

    await u.click(toggle);
    expect(screen.getByRole("checkbox", { name: "web-app" })).toBeInTheDocument();

    await u.click(toggle);
    expect(screen.queryByRole("checkbox", { name: "web-app" })).toBeNull();
  });
});
