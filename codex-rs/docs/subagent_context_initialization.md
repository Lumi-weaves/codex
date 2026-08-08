# Lumi and subagent context initialization

This note records how this downstream branch initializes model context for the
root Lumi session and for a fresh `deepseek_worker`. It is intentionally a
request-layout document rather than a general prompting guide: the goal is to
make prompt ownership, precedence, and physical placement inspectable.

The source/deployment snapshot described here was verified on 2026-08-08 at
commit `261ce781a8fde11b093fe2883c6caae132b1d140`. Runtime configuration can
change independently of this repository, so the "current deployment" facts
below must be rechecked when the role or personal environment changes.

The Desktop root session used during this investigation was older than that
snapshot: its long-lived app-server process was running commit
`32c173a6352541dd4e670bb8c4845ad39fade06c`, while the installed CLI symlink had
already advanced to `261ce781...`. The running root retained the base
instructions and initial context persisted when its rollout began. The diagrams
below therefore describe a **fresh session on the named source/deployment
snapshot**; the following runtime-drift section records the observed live root.

## Executive result

For a fresh root and a V2 spawn with `agent_type = "deepseek_worker"` and
`fork_turns = "none"` under the same current configuration:

- root Lumi and the DeepSeek worker currently resolve to the **same
  base-instruction bytes**, but by different lifetimes: root retains its
  session-resolved base, while role reload reconstructs the child config and
  rereads the configured `model_instructions_file`;
- the DeepSeek role supplies no separate base prompt. It replaces the child
  `developer_instructions` slot with its bounded-worker policy, while role
  reload causes base instructions to be resolved again from current layers;
- root Lumi receives the downstream branch's **root** MultiAgentV2 usage hint;
- a thread-spawned DeepSeek worker receives the downstream branch's
  **subagent** MultiAgentV2 usage hint;
- both see the same currently visible skill catalog, although the model-derived
  catalog budgets differ; and
- the initial DeepSeek task is a plaintext inter-agent message converted to a
  normal `user` message for the non-OpenAI provider.

The phrase "base instructions" is easy to misuse here. It identifies a logical
request component. Its wire placement differs by model protocol:

- root `gpt-5.6-sol` uses Responses Lite, so the base text is physically
  inserted as a leading `developer` input item;
- `deepseek-v4-flash` uses the normal Responses request, so the same base text
  is physically sent in the top-level `instructions` field.

## Current deployment inputs

The durable personal-environment source lives outside this repository. The
installed values relevant to this layout are:

| Input | Root Lumi | Fresh DeepSeek worker |
| --- | --- | --- |
| Model | `gpt-5.6-sol` | `deepseek-v4-flash` |
| Provider | root configured provider | `deepseek` |
| Reasoning effort | `medium` | `max` |
| Model context window | 272,000 catalog tokens | 1,000,000 role/catalog tokens |
| Base-instruction source | `$CODEX_HOME/lumi.md` | reread from `$CODEX_HOME/lumi.md` during role reload; equal while the configured bytes remain unchanged |
| Developer instructions | client/config value, if any | role-local bounded-worker policy |
| Multi-agent guide | root usage hint | subagent usage hint |
| Active multi-agent mode overlay | absent: configured custom text is empty | absent for the same reason |
| Skills catalog | current 23 visible entries | the same 23 visible entries |

## Live-process drift observed during the investigation

The live Desktop root was created by the older `32c173...` app-server and its
rollout retains the older `lumi.md` revision captured at thread start. Its
actual first Responses Lite request had this input layout:

```text
input[0]  developer AdditionalTools
input[1]  developer persisted older Lumi BaseInstructions
input[2]  developer aggregate:
          Desktop app context, skills, permissions,
          Default collaboration mode, apps, plugins
input[3]  developer older downstream ROOT MultiAgentV2 hint
input[4]  user aggregate: recommended plugins, environment
input[5]  user: Fletcher's first request
```

The world-state and turn-context snapshots persisted beside those items are
bookkeeping baselines, not additional model input rows. Compared with current
HEAD, the live root's generated usage hint also predates the added long-wait
guidance.

More importantly, attempting to spawn the **current installed**
`deepseek_worker` role from that old live process fails before child creation:

1. `32c173...` expects the old model-catalog schema with a required
   `base_instructions` field.
2. The current role-local catalog uses the new schema with
   `model_messages: null` and no `base_instructions` field.
3. Role reload parses the current file with the old binary and fails; the
   surfaced error is `agent type is currently not available`.

Thus, in the still-running Desktop task there is **no actual child base prompt
or first DeepSeek request**. If the catalog were parse-compatible, the old role
reload would discard the copied runtime-only parent base override and reread
the current `model_instructions_file`, so the prospective child would receive
the newer installed `lumi.md`, not the root rollout's persisted older copy.

The old process also predates the per-call role-catalog lookup fix. Even with an
old-schema-compatible catalog it would ignore the role-local 1M model metadata,
fall back to the unknown-model descriptor, and clamp the effective window to
272K. Restarting the Desktop/app-server onto `261ce781...` is therefore part of
making the current-HEAD diagram operational, not merely cosmetic process
hygiene.

## Base-instruction resolution

The installed configuration selects `lumi.md` with
`model_instructions_file`. `Config::load_config_with_layer_stack` reads it into
`Config.base_instructions` in `core/src/config/mod.rs`. Session startup resolves
base instructions in this order in `core/src/session/mod.rs`:

1. `config.base_instructions`;
2. persisted rollout/session base instructions;
3. the selected model's `model_messages.instructions_template`.

`build_agent_spawn_config` in
`core/src/tools/handlers/multi_agents_common.rs` first pins the live parent's
resolved `BaseInstructions` into the prospective child config. Applying the
role reconstructs `Config` from the layered TOML and does not preserve that
runtime-only override; the loader then re-reads the still-configured
`model_instructions_file`. In the current deployment both routes resolve to the
same installed `lumi.md` bytes. The result is exact equality, but it depends on
the role retaining that config layer rather than on DeepSeek's model metadata.

The current DeepSeek catalog deliberately has `model_messages: null`. On a
fresh child there is also no previous model identity, so
`ModelInstructionsState` in `core/src/context/world_state/model.rs` does not
emit a model-switch instruction fragment. Consequently, no second upstream
Codex base prompt is added on this path.

## MultiAgentV2 guide selection

The live guide is not an upstream opaque "harness guide". In this branch it is
assembled from constants in `core/src/config/mod.rs`:

- `DEFAULT_MULTI_AGENT_V2_ROOT_AGENT_USAGE_HINT_TEXT`;
- `DEFAULT_MULTI_AGENT_V2_SUBAGENT_USAGE_HINT_TEXT`;
- the shared filesystem/tool-routing text;
- optional `wait_agent` guidance;
- the configured concurrency count; and
- the model/fork override suffix when model overrides are exposed.

The current personal configuration changes the namespace to
`lumi_collaboration`, sets the maximum concurrency to nine, and sets
`multi_agent_mode_hint_text = ""`. It does not override either usage-hint
body, so root and child receive the **defaults compiled into this downstream
branch**, not the corresponding upstream release text.

`session/multi_agents.rs::configured_usage_hint_text_for_source` chooses the
variant:

- CLI, VS Code, Exec, MCP, custom, and unknown root-like sources get the root
  hint;
- `SessionSource::SubAgent(ThreadSpawn { .. })` gets the subagent hint;
- other internal subagents get no V2 usage hint.

`MultiAgentUsageHint` requires a standalone `developer` message. The configured
empty multi-agent mode is intentionally rendered as no message, so there is no
later explicit-request-only or proactive-mode overlay in the current setup.

### Known policy collision

The current `deepseek_worker` role and `lumi.md` both define small workers as
leaf agents that must not spawn or coordinate other agents. The current
downstream **subagent usage hint**, however, still says that a subagent can
spawn and manage further subagents.

These are not merely comments in different subsystems. The role policy enters
the aggregate developer context and the usage hint is emitted later as a
standalone developer message. This is a real model-visible conflict. In
addition, the current DeepSeek model catalog does not declare
`multi_agent_version = "v2"`, so `spec_plan.rs::collab_tools_enabled` withholds
collaboration tools from the spawned worker. The guide therefore advertises
capabilities the child does not receive. Its shared example also names
`functions.collaboration.spawn_agent` while the deployment namespace is
`lumi_collaboration`.

The role's leaf policy agrees with the actual tool surface; the later usage
hint does not. This should be resolved deliberately in the subagent usage-hint
design rather than assumed away.

## Context as a physical request layout

The diagrams use increasing addresses to mean increasing position in the
serialized request input. The addresses are illustrative, not byte offsets.
Separate request fields are shown as control registers outside the input
vector, like memory-mapped control state.

### Fresh root Lumi on current HEAD: first request on Responses Lite

```text
           REQUEST CONTROL REGISTERS
  +------------------------------------------------------------+
  | MODEL        gpt-5.6-sol                                   |
  | instructions ""                                            |
  | tools        None (encoded into input[0])                  |
  +------------------------------------------------------------+

           INPUT VECTOR                         low -> high
  0x0000  +----------------------------------------------------+
          | developer: AdditionalTools                         |
          | Tool schemas, including lumi_collaboration         |
  0x1000  +----------------------------------------------------+
          | developer: BaseInstructions                        |
          | Exact resolved bytes from $CODEX_HOME/lumi.md      |
  0x2000  +----------------------------------------------------+
          | developer: aggregate initial context               |
          | external Desktop <app-context>, when supplied      |
          | thread/turn developer extension contributions      |
          | <skills_instructions> catalog                      |
          | permissions + conditional collaboration guidance  |
          | conditional environment/apps/deferred-tool guide   |
          | no generic plugins guide with current metadata     |
  0x3000  +----------------------------------------------------+
          | developer: standalone ROOT MultiAgentV2 hint       |
          | root + shared + wait_agent + 9 slots + fork suffix |
  0x4000  +----------------------------------------------------+
          | developer: active multi-agent mode                 |
          | ABSENT because custom mode text is empty           |
  0x5000  +----------------------------------------------------+
          | user: aggregate contextual-user message            |
          | recommended plugins + AGENTS.md when loaded        |
          | + <environment_context>                            |
  0x6000  +----------------------------------------------------+
          | user: the actual root user input                   |
  0x7000  +----------------------------------------------------+
          | user: selected <skill> bodies, when triggered      |
          | recorded after the triggering user input           |
  0x8000  +----------------------------------------------------+
          | later retained history / turn output               |
          +----------------------------------------------------+
```

The aggregate rows are conditional: empty or unavailable sections do not
occupy an item. `Session::build_initial_context_with_world_state` in
`core/src/session/mod.rs` owns their assembly and exact item ordering.

### Fresh DeepSeek worker: first normal Responses request

```text
           REQUEST CONTROL REGISTERS
  +------------------------------------------------------------+
  | MODEL        deepseek-v4-flash                             |
  | PROVIDER     deepseek                                      |
  | instructions exact bytes reread from the currently         |
  |              configured $CODEX_HOME/lumi.md                |
  | tools        top-level Responses tool schemas              |
  |              (no collaboration tools in current catalog)  |
  +------------------------------------------------------------+

           INPUT VECTOR                         low -> high
  0x0000  +----------------------------------------------------+
          | developer: aggregate initial context               |
          | DeepSeek role bounded-worker instructions FIRST    |
          | thread/turn extension contributions                |
          | <skills_instructions> catalogs                     |
          | permissions instructions                           |
          | apps guide when an accessible enabled app exists   |
          | optional extension state such as git attribution   |
          | no model-owned collaboration-mode fragment         |
          | no generic plugins guide with current flags        |
          | replaces root Desktop <app-context>/developer slot |
  0x1000  +----------------------------------------------------+
          | developer: standalone SUBAGENT MultiAgentV2 hint   |
          | child + shared + wait_agent + 9 slots + fork suffix|
  0x2000  +----------------------------------------------------+
          | developer: active multi-agent mode                 |
          | ABSENT because custom mode text is empty           |
  0x3000  +----------------------------------------------------+
          | user: aggregate contextual-user message            |
          | recommended plugins + AGENTS.md freshly loaded     |
          | for child cwd, when applicable                     |
          | + fresh child <environment_context>                |
  0x4000  +----------------------------------------------------+
          | user: NEW_TASK envelope                            |
          | AgentMessage converted to user for non-OpenAI wire |
  0x5000  +----------------------------------------------------+
          | no inherited transcript for fork_turns="none"      |
          +----------------------------------------------------+
```

The logical NEW_TASK is constructed as `InterAgentCommunication` and recorded
as `ResponseItem::AgentMessage`. `client.rs::build_responses_request` converts
an all-plaintext agent message to a normal `user` message whenever the child
provider is not OpenAI. Its position is unchanged by that conversion.

The Desktop `<app-context>` is supplied through the root thread's external
`developer_instructions`. Applying `deepseek_worker` replaces that slot with
the role-local bounded-worker policy, so the fresh worker does not also receive
the root Desktop app-context block.

## Skills: same catalog, one important trigger difference

The root and fresh DeepSeek worker currently resolve the same 23 visible skill
entries. The role does not override skill rules, plugin configuration, cwd, or
skill roots. Both models also omit the long skills-usage footer.

Skill rendering has two budget paths:

- the thread-context bundled/plugin catalog uses the fixed 8,000-character
  budget for both models;
- world-state executor/orchestrator/host catalogs use 2% of the resolved model
  context window: 5,440 tokens for root Lumi and 20,000 tokens for the DeepSeek
  worker.

The current entries fit the applicable budgets, so membership is presently the
same. A larger future world-state catalog could make the model-window
difference observable.

Available-skills context contains metadata, not every `SKILL.md` body. A normal
root `UserInput` can trigger full selected-skill injection after the triggering
input is recorded and before sampling.
The initial child task cannot: `turn_user_input` in `core/src/session/turn.rs`
deliberately excludes `InterAgentCommunication`. Therefore naming `$skill` only
inside `spawn_agent.message` does not inject its body. A fresh worker must read
the locator exposed in the catalog, or the task must explicitly tell it to read
the exact `SKILL.md`.

## Fork mode changes transcript inheritance and can change final base resolution

| `fork_turns` | Copied transcript | Context baseline | Role override |
| --- | --- | --- | --- |
| `"none"` | none | rebuilt completely | allowed |
| positive integer | filtered recent turns | dropped, then rebuilt | allowed |
| omitted / `"all"` | filtered full history | preserved and diffed | `agent_type` rejected |

Spawn construction initially seeds every child config from the parent session's
resolved base. For `"none"` and bounded forks, role application may reconstruct
config and resolve the base again from current layers; the current DeepSeek
role rereads `model_instructions_file`. Full-history forks reject `agent_type`
and retain the parent-derived base/history path.

## Source map

- Spawn and NEW_TASK construction:
  `core/src/tools/handlers/multi_agents_v2/spawn.rs`
- Parent-to-child base/config copy:
  `core/src/tools/handlers/multi_agents_common.rs`
- Role layer and developer-instruction precedence:
  `core/src/agent/role.rs`
- Session base-instruction resolution:
  `core/src/session/mod.rs`
- Initial context item assembly:
  `core/src/session/mod.rs::build_initial_context_with_world_state`
- World-state ordering and gates:
  `core/src/session/world_state.rs`
- Root/subagent usage-hint selection:
  `core/src/session/multi_agents.rs`
- V2 default/override assembly:
  `core/src/config/mod.rs::resolve_multi_agent_v2_config`
- Responses Lite and non-OpenAI request transformation:
  `core/src/client.rs::build_responses_request`
- Skills catalog and body contribution:
  `ext/skills/src/extension.rs`, `ext/skills/src/world_state_catalogs.rs`
- Skill selection exclusion for initial inter-agent messages:
  `core/src/session/turn.rs::turn_user_input`

## Verification checklist after configuration changes

1. Resolve the root `model_instructions_file` and compare the installed bytes
   with their durable personal-environment source.
2. Inspect role-local `model`, provider, catalog, context-window, developer,
   skills, and plugin overrides.
3. Resolve `use_responses_lite` and the apps/plugins/skills flags from the
   selected per-session model catalog.
4. Re-render root and subagent V2 hints using the configured concurrency and
   override fields; do not compare against upstream prose by memory.
5. Check whether a non-empty active multi-agent mode adds a later developer
   override.
6. Compare the rendered skills catalog against each model's metadata budget.
7. Inspect the final provider-specific request representation, especially
   `AgentMessage` conversion and base/tool placement.
