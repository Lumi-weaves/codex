import { Link, Outlet } from "@tanstack/react-router";

export interface ShellNavItem {
  label: string;
  description: string;
  /** Present only for implemented routes; absent means "coming soon". */
  to?: string;
}

export interface ShellNavSection {
  title: string;
  items: ShellNavItem[];
}

export const SHELL_NAV_SECTIONS: ShellNavSection[] = [
  {
    title: "Operations",
    items: [
      {
        label: "Agent Operations",
        to: "/agent-operations",
        description: "Causal timeline of generations, async work, and joins",
      },
    ],
  },
  {
    title: "Coming soon",
    items: [
      { label: "Prompt Studio", description: "Draft and compare prompts" },
      { label: "Providers", description: "Model provider configuration" },
      { label: "Profiles", description: "Agent configuration profiles" },
      { label: "Releases", description: "Release train status" },
      { label: "Hosts", description: "Connected hosts and fleets" },
    ],
  },
];

export function ShellLayout() {
  return (
    <div className="shell">
      <aside className="shell__sidebar">
        <div className="shell__brand">
          <span className="shell__brand-mark" aria-hidden="true" />
          <div>
            <div className="shell__brand-name">Lumi Codex</div>
            <div className="shell__brand-sub">Local console</div>
          </div>
        </div>
        <nav aria-label="Primary">
          {SHELL_NAV_SECTIONS.map((section) => (
            <div className="shell__section" key={section.title}>
              <h2 className="shell__section-title">{section.title}</h2>
              <ul className="shell__nav">
                {section.items.map((item) => (
                  <li key={item.label}>
                    {item.to ? (
                      <Link
                        to={item.to}
                        className="shell__link"
                        activeProps={{
                          "className": "shell__link shell__link--active",
                          "aria-current": "page",
                        }}
                        title={item.description}
                      >
                        {item.label}
                      </Link>
                    ) : (
                      <span
                        className="shell__link shell__link--disabled"
                        aria-disabled="true"
                        title={`${item.description}. Not available yet.`}
                      >
                        {item.label}
                        <span className="shell__badge">Soon</span>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </nav>
        <p className="shell__footnote">Read-only shell · no mutations</p>
      </aside>
      <main className="shell__content">
        <Outlet />
      </main>
    </div>
  );
}
