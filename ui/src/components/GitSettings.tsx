import { useAgentSettings } from "../lib/queries";
import { useUi } from "../lib/store";
import { Group } from "./GeneralSettings";
import { Checkbox, Field, Select } from "./ui";

export function GitSettings() {
  const { data: agents } = useAgentSettings();
  const gitTaskProfile = useUi((s) => s.gitTaskProfile);
  const setGitTaskProfile = useUi((s) => s.setGitTaskProfile);
  const showCommits = useUi((s) => s.showCommits);
  const setShowCommits = useUi((s) => s.setShowCommits);
  const showPullRequests = useUi((s) => s.showPullRequests);
  const setShowPullRequests = useUi((s) => s.setShowPullRequests);

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">Git</h2>
      <div className="flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-6 pb-6">
        <Group title="Features">
          <Checkbox checked={showCommits} onChange={setShowCommits}>
            <span className="text-[13px]">Commits</span>
            <span className="block text-xs text-faint">
              Commit box, the list of commits and rebasing in the changes panel.
            </span>
          </Checkbox>
          <Checkbox checked={showPullRequests} onChange={setShowPullRequests}>
            <span className="text-[13px]">Pull requests</span>
            <span className="block text-xs text-faint">
              Creating pull requests and showing their status and checks. Uses the GitHub CLI (gh).
            </span>
          </Checkbox>
        </Group>

        <Group title="Agent tasks">
          <Field
            label="Agent for “Ask agent” buttons"
            hint="Committing, resolving conflicts and similar tasks. A separate agent, e.g. a profile running claude --model haiku, starts in its own tab and keeps your main conversation (and its model) untouched."
          >
            <Select
              value={gitTaskProfile ?? ""}
              onChange={(e) => setGitTaskProfile(e.target.value || null)}
              className="h-8 max-w-72 text-[13px]"
            >
              <option value="">The worktree's open agent</option>
              {(agents?.profiles ?? []).map((p) => (
                <option key={p.id} value={p.id}>
                  New tab: {p.name}
                </option>
              ))}
            </Select>
          </Field>
        </Group>
      </div>
    </section>
  );
}
