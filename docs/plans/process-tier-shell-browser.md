# Plan: Process-Tier Shell and Browser Support

Status: Proposed  
Created: 2026-03-06

## Executive summary

Today, `process` tier hard-disables both shell and browser in multiple places. The current implementation assumes privileged tools are only available when the runner also launches a sidecar guest, and `process` tier does not do that today.

There are two serious ways to enable shell and browser in `process` tier:

1. **Hybrid process runtime + existing `shell-vm` sidecar container**  
   Keep `oxydra-vm` running as a host process, but launch the existing `shell-vm` sidecar container for shell/browser. This is the **lowest-risk, highest-parity** path and should be the default recommendation if using Docker/OCI in process tier is acceptable.
2. **Pure host-local process tier**  
   Use host-local shell execution and host-local browser/CDP plumbing without any sidecar container. This is the **truest interpretation of “process tier”**, but it is a materially larger security, packaging, and productization effort.

**Recommendation:** if the goal is to ship shell+browser in process tier with the least risk and the best UX consistency, use the **hybrid sidecar-container design** first. If the product requirement is specifically “no Docker/container dependency whatsoever in process tier”, treat that as a separate, larger program and do not frame it as a small extension of the current architecture.

## What “restrictive-only overrides” means

The current config model after the recent cleanup is:

- `agent.toml` decides the global defaults:
  - `tools.shell.enabled`
  - `tools.browser.enabled`
- `runner-user.toml` can only apply per-user restrictions:
  - `behavior.shell_enabled`
  - `behavior.browser_enabled`

Effective tool access is conceptually:

```text
effective_shell =
  tier_supports_shell
  AND agent.tools.shell.enabled
  AND user.behavior.shell_enabled.unwrap_or(true)

effective_browser =
  tier_supports_browser
  AND agent.tools.browser.enabled
  AND user.behavior.browser_enabled.unwrap_or(true)
```

So:

- if the agent/global config disables shell/browser, a user cannot force-enable it
- if the sandbox tier disables shell/browser, a user cannot force-enable it
- if the user sets `behavior.shell_enabled = false`, that user loses shell even if it is globally allowed
- if the user sets `behavior.shell_enabled = true`, that only says “do not restrict me further”; it does **not** grant access by itself

That is why these fields are “restrictive-only overrides”.

## Current state

### Capability resolution

`crates/runner/src/lib.rs` currently resolves requested capabilities and forces both shell and browser off in `SandboxTier::Process`. This is the first hard gate.

### Backend launch behavior

`crates/runner/src/backend.rs`:

- `launch_process()` launches only the host `oxydra-vm` process
- returns `sidecar_endpoint: None`
- returns `shell_available: false`
- returns `browser_available: false`

So even if higher-level config wanted shell/browser, the backend cannot currently satisfy that request in process tier.

### Tool bootstrap behavior

`crates/tools/src/lib.rs` bootstraps shell/browser through the sidecar connection model:

- if there is no sidecar endpoint, shell/browser bootstrap as unavailable
- process tier gets a process-specific disabled reason today

### Runtime registration and UX copy

Several surfaces assume process tier means “no shell/browser”:

- `crates/tools/src/registry.rs` startup/degraded reporting
- `crates/runner/src/bootstrap.rs` system prompt copy
- README/docs/web copy that describe browser as unavailable in process tier

### Browser architecture today

Current browser support is not an isolated CDP client. It is a small stack:

- shell/browser sidecar guest
- shell-daemon endpoint
- shared shell session
- browser tool
- BrowserAutomation skill
- Pinchtab/Chromium lifecycle in the shell-vm image
- browser-specific shell policy overlay (`curl`, `jq`, `sleep`, `allow_operators`)

This is important: **browser support is coupled to the existing shell sidecar model**, not just to a single `cdp_url`.

### Process-tier primitives that already exist

There is useful prior art for a host-local path:

- `LocalProcessShellSession` already exists in `crates/tools/src/sandbox/mod.rs`
- `run_shell_command()` already supports host-local shell execution with cwd/env/timeout support
- process-tier hardening attempts exist, but only as best-effort probes/logging today

Important limitation in the current code:

- `spawn_process_guest()` and `spawn_process_guest_with_startup_stdin()` do **not** currently inject `request.extra_env` / `request.shell_env` into the host `oxydra-vm` process path
- browser shell overlay currently mutates the workspace agent config file in place via `write_shell_overlay()`, which is more awkward once the runtime is a host process rather than a container/microVM bootstrap path

## Goals

1. Enable shell and browser in process tier without muddying config ownership.
2. Preserve the new global-vs-user config model:
   - operator/global defaults in `agent.toml`
   - per-user restrictions in `runner-user.toml`
3. Keep process-tier enablement behind an explicit deployment/operator decision.
4. Preserve stable tool contracts where possible.
5. Make status, warnings, onboarding, and web configuration explain the real security/runtime implications.
6. Avoid silent fallbacks or ambiguous partial enablement.

## Non-goals

1. Making process tier as strong as microVM isolation.
2. Hiding the fact that process-tier shell/browser are materially less isolated.
3. Forcing all users of process tier to adopt shell/browser.
4. Solving every future host-native browser architecture problem in the first rollout.

## Architecture options

### Option A: Hybrid process runtime + existing `shell-vm` sidecar container

#### Summary

- `oxydra-vm` runs as a host process, as process tier does today
- shell/browser are provided by the existing `shell-vm` sidecar container
- tools continue using the current sidecar endpoint contract

#### Why this fits the current repo well

It preserves the current privileged-tool architecture:

- shell/browser still live behind a sidecar boundary
- browser still uses the same Pinchtab/Chromium stack
- tool bootstrap and tool contracts change less
- BrowserAutomation skill keeps working with minimal conceptual drift

#### Required changes

#### 1. Add an explicit process-tier privileged-tools backend setting

This should **not** live under user behavior and should **not** live under agent tool config.

It is a deployment/runtime backend decision, so it belongs in **runner global config**. A minimal shape:

```toml
[process_tier]
privileged_tools_backend = "disabled" # disabled | container_sidecar | host_local
```

Why runner config:

- it is deployment-wide
- it changes runtime topology and security posture
- it is not user-scoped
- it is not just a tool default; it decides how process tier is allowed to realize those tools

#### 2. Change process-tier capability resolution

`resolve_requested_capabilities()` should no longer blindly disable shell/browser on `SandboxTier::Process`.

Instead:

- if backend = `disabled`, current behavior stays
- if backend = `container_sidecar`, process tier may request shell/browser
- the existing `agent.toml` + per-user restrictive override logic still applies on top

#### 3. Teach `launch_process()` to optionally launch the sidecar container

`crates/runner/src/backend.rs` needs a new branch:

- launch host `oxydra-vm`
- if `request.sidecar_requested()` and backend = `container_sidecar`, launch `shell-vm` container using the existing Docker path
- return a Unix `sidecar_endpoint`
- report actual `shell_available` / `browser_available`

This is the core architecture change.

#### 4. Inject env into the host `oxydra-vm` process path

Today, process-tier host spawn does not inject `request.extra_env` or `request.shell_env`. That must be fixed for parity with the container path, otherwise process tier will diverge on:

- config-referenced API keys
- CLI-provided env vars
- shell tool env keys

#### 5. Refactor browser shell overlay handling

Today `write_shell_overlay()` mutates the workspace agent config file. That is fragile for this design.

Before or during process-tier support, prefer moving browser shell policy augmentation to an **explicit computed effective config** instead of file mutation. For example:

- compute effective shell policy in memory
- pass it into runtime bootstrap/tool registration directly
- avoid “rewrite a config file so another component picks it up” coupling

This cleanup is valuable even outside process tier.

#### 6. Update status, prompt, and web/UI text to be backend-aware

Current UX assumes:

- process tier => shell disabled
- process tier => browser disabled

That must become:

- process tier + backend disabled => shell/browser disabled
- process tier + backend enabled => shell/browser available, but with strong warnings about security/runtime requirements

#### 7. Add preflight checks

When backend = `container_sidecar`, startup should preflight:

- Docker/OCI availability
- image availability/pullability
- sidecar launch success
- browser prerequisites if browser requested

Failures should be surfaced as explicit degraded reasons, not generic “tool unavailable”.

#### Pros

- Lowest code churn for browser
- Highest behavioral parity with existing container/microVM tool flows
- Reuses current shell-daemon, BrowserTool, Pinchtab, and image lifecycle
- Safer than host-local direct execution
- Easier to test because the existing sidecar model already exists

#### Cons

- “Process tier” no longer means “no Docker involvement” if privileged tools are enabled
- Mixed topology: host runtime + sidecar container
- Requires careful UX so users understand why process tier may still need Docker

## Option B: Pure host-local process tier

#### Summary

- `oxydra-vm` runs as a host process
- shell runs directly on the host via `LocalProcessShellSession`
- browser is implemented via host-local CDP/Pinchtab/Chromium support
- no sidecar container is involved

#### Shell feasibility

Shell is very feasible here because the repo already contains the core primitive:

- `LocalProcessShellSession`

The remaining work is not the shell session itself. The hard parts are:

- stronger host hardening
- env scrubbing/policy parity
- cwd/path semantics
- operator UX and warnings

#### Browser feasibility

Browser is the harder half by far. There are three real sub-options:

1. **External CDP only**  
   Require `tools.browser.cdp_url` and do not manage Chrome locally. Lowest implementation cost, weakest out-of-box UX.
2. **Local Pinchtab + host Chrome/Chromium manager**  
   Recreate the current shell-vm browser behavior on the host. Stronger UX, much larger packaging/lifecycle effort.
3. **New native browser automation stack**  
   Replace more of the current architecture. Highest churn, least desirable for a first rollout.

#### Additional work unique to this option

#### 1. Real hardening model

`attempt_process_tier_hardening()` is not enough to support host-local privileged tools as a product claim. We would need a real statement of what is enforced:

- filesystem limits
- network rules
- child-process restrictions
- OS support matrix
- what happens when hardening is unavailable

Without that, the product story is effectively “run shell/browser directly on the host with best-effort warnings”.

#### 2. Browser lifecycle management

Need host-local management of:

- browser binary detection or installation guidance
- Pinchtab or equivalent bridge lifecycle
- port/socket allocation
- profile directories and cleanup
- crash recovery
- health checks

#### 3. Path and workflow semantics

The current browser stack expects container-style shared paths and a known sidecar environment. Host-local browser support would need an explicit policy for:

- file upload/download paths
- skill documentation
- log collection
- where browser state lives

#### Pros

- True Docker-free process tier
- Most honest interpretation of the tier name
- Shell support can reuse existing local-session primitives

#### Cons

- Largest security exposure
- Browser is much harder than shell
- More packaging/OS-specific behavior
- More behavior drift from existing container/microVM paths
- More docs/onboarding/support burden

## Option C: Host-native sidecar preserving the current tool contract

#### Summary

- keep the sidecar contract
- replace the sidecar container with host-native services/processes
- run a local shell-daemon and local browser bridge that expose the same endpoint contract

#### Why it is interesting

This keeps the tool contract closer to the current design than Option B while still avoiding Docker.

#### Why it is not the first recommendation

It is still a large program:

- service supervision
- endpoint lifecycle
- host hardening
- browser lifecycle
- packaging per OS

It is better thought of as a future evolution if process tier becomes a major product mode.

## Option comparison

| Option | Shell effort | Browser effort | Security posture | Behavior parity | Docker-free | Recommended use |
| --- | --- | --- | --- | --- | --- | --- |
| A. Process + container sidecar | M | M | Best of the three | High | No | Default recommendation |
| B. Pure host-local | M | XL | Weakest | Low-Medium | Yes | Only if no Docker is a hard requirement |
| C. Host-native sidecar | L | XL | Medium | Medium-High | Yes | Future investment, not first rollout |

## Recommended direction

### Primary recommendation

Implement **Option A** first:

- add an explicit runner-level process-tier privileged-tools backend
- support `container_sidecar`
- keep tool/config semantics unchanged above the backend layer

This gives the best user experience because shell/browser work mostly like they already do outside process tier.

### Strategic caveat

If the real product requirement is:

> “Process tier must remain completely free of Docker/container dependencies, even when shell/browser are enabled.”

then **do not** choose Option A. In that case, choose Option B and treat it as a larger, security-sensitive project with a different implementation and rollout shape.

## Proposed configuration model

### Operator/deployment scope

Add runner-global config:

```toml
[process_tier]
privileged_tools_backend = "disabled" # disabled | container_sidecar | host_local
```

Potential future extension:

```toml
[process_tier]
privileged_tools_backend = "container_sidecar"
require_explicit_warning_ack = true
```

### Agent scope

Keep global tool defaults in `agent.toml`:

```toml
[tools.shell]
enabled = true

[tools.browser]
enabled = true
cdp_url = "..."
```

### User scope

Keep per-user restrictive overrides in `runner-user.toml`:

```toml
[behavior]
shell_enabled = false
browser_enabled = false
```

### Why this split is correct

- runner config decides whether process tier is even allowed to realize privileged tools and by what backend
- agent config decides global tool defaults/policy
- user config can only narrow access for that user

This preserves ownership boundaries cleanly.

## Detailed code changes for the recommended path

### `crates/types`

Add a runner-global process-tier section and validation:

- new config type for process-tier backend mode
- parse/validate supported enum values
- schema/help text describing security and Docker implications

Potentially add new startup/degraded reason codes, for example:

- `ProcessTierPrivilegedToolsDisabled`
- `ProcessTierPrivilegedToolsBackendUnavailable`
- `ProcessTierPrivilegedToolsActive`

The exact names can change, but the important point is to distinguish:

- disabled by policy
- requested but backend failed
- enabled with warnings

### `crates/runner/src/lib.rs`

Refactor capability resolution so it depends on:

- sandbox tier
- runner process-tier backend mode
- agent tool config
- per-user restrictive override

Also:

- stop keying prompt/status copy purely off `SandboxTier::Process`
- make browser provisioning conditional on actual requested browser support, not just non-process tier
- refactor browser shell overlay from file mutation to effective in-memory config if possible

### `crates/runner/src/backend.rs`

For `launch_process()`:

- keep host-process `oxydra-vm`
- add optional `shell-vm` container launch when backend = `container_sidecar`
- inject `request.extra_env` into host runtime spawn
- ensure `request.shell_env` reaches the sidecar path
- return actual sidecar endpoint and availability booleans
- add backend-specific degraded reasons/warnings

### `crates/tools/src/lib.rs`

Minimal changes should be needed if a valid sidecar endpoint exists. Likely work items:

- update process-tier-specific disabled messages so they are conditional on actual backend state
- preserve the current sidecar bootstrap path for the hybrid design

### `crates/tools/src/registry.rs`

Startup status currently assumes process tier implies disabled privileged tools. That must become backend-aware so the UI and CLI reflect real availability.

### `crates/runner/src/bootstrap.rs`

Update system prompt generation so it says what is actually true:

- disabled if backend/policy says disabled
- enabled with warnings if process tier privileged tools are active

The prompt should explicitly mention that process-tier shell/browser have weaker isolation than container/microVM modes.

### `crates/runner/src/web/schema.rs`

Add a new runner-global web configurator section for process-tier privileged tools:

- backend selector
- warning/help copy
- validation hints

Do **not** move this into user behavior or agent tools.

### `crates/runner/static/js/*`

Update:

- onboarding copy
- review summary
- effective tool explanations
- degraded status display

The web configurator should clearly show the three layers:

1. process-tier backend capability
2. agent/global tool enablement
3. per-user restriction

### Docs and examples

Update:

- README
- guidebook chapters
- example configs
- onboarding docs

Especially important:

- explain why process tier can still require Docker if `container_sidecar` is chosen
- explain security differences across tiers/backends

## UX requirements for a good rollout

This work is not just backend plumbing. Good UX requires the following.

### 1. Show effective availability and the reason

The configurator should not only show raw config. It should explain:

- shell globally enabled: yes/no
- browser globally enabled: yes/no
- process-tier backend: disabled/container-sidecar/host-local
- user restriction applied: yes/no
- final effective availability: yes/no

Without this, operators will struggle to understand why a tool is unavailable.

### 2. Strong warnings at the point of enablement

When enabling process-tier privileged tools, the UI should show a direct warning:

- weaker isolation than microVM/container
- possible Docker dependency for `container_sidecar`
- browser is a heavier dependency than shell

### 3. Preflight/doctor output

Before or during startup, surface actionable failures such as:

- Docker missing
- sidecar image missing
- browser bridge failed health check
- `cdp_url` misconfigured

This should appear in CLI and web status, not just logs.

### 4. Better status fields

Expose more structured startup state, for example:

- `process_tier_privileged_tools_backend`
- `shell_backend_active`
- `browser_backend_active`
- `shell_unavailable_reason`
- `browser_unavailable_reason`

### 5. Keep onboarding simple

Do not overload onboarding with every advanced knob. The onboarding flow should ask:

- do you want shell in process tier?
- do you want browser in process tier?
- if yes, which backend mode is supported here?

Then link advanced config to the web configurator/manual files.

### 6. Use explicit naming

Avoid vague labels like “advanced shell mode”. Use names that describe the real topology:

- Disabled
- Process runtime + sidecar container
- Host-local execution

## Phased rollout

### Phase 0: Decision and schema

Deliverables:

- decide whether Docker-in-process-tier is acceptable
- add runner-global config schema for process-tier privileged-tools backend
- add backend-aware status model

Gate:

- clear product decision on whether Option A is acceptable

### Phase 1: Shared prerequisites

Deliverables:

- refactor capability resolution away from “process tier always false”
- inject env into host process launch path
- refactor browser shell overlay handling toward effective config composition
- update prompt/status copy to be backend-aware

Gate:

- no backend behavior change yet, but the model can represent it cleanly

### Phase 2A: Hybrid sidecar-container MVP

Deliverables:

- `launch_process()` can launch `shell-vm` sidecar container
- sidecar endpoint returned in process tier
- shell tool available in process tier behind explicit config
- browser optional but can stay feature-gated until health checks are solid

Gate:

- shell works end-to-end in process tier with sidecar container
- failures are surfaced clearly

### Phase 3A: Browser on hybrid backend

Deliverables:

- browser tool works end-to-end in process tier on sidecar container backend
- web/onboarding/docs updated
- BrowserAutomation skill/path semantics validated

Gate:

- browser open/navigate/screenshot/evaluate flows are reliable

### Phase 4A: Hardening and UX polish

Deliverables:

- structured warnings/degraded reasons
- preflight diagnostics
- effective availability view in web configurator
- docs/examples polished

Gate:

- operators can understand and support the feature without reading code

### Optional Phase 2B+: Host-local R&D track

If Docker-free process tier is a hard requirement:

- build a separate track for host-local shell/browser
- likely ship shell before browser
- treat browser as a distinct subproject with its own OS matrix and hardening story

## Testing and validation

### Unit/integration coverage

Need tests for:

- capability resolution matrix
- config validation for new process-tier backend settings
- startup status/degraded reasons
- prompt copy
- web schema generation

### End-to-end coverage

For the hybrid design:

- process tier + backend disabled
- process tier + backend enabled + shell only
- process tier + backend enabled + shell + browser
- Docker unavailable
- sidecar launch failure
- browser bridge failure
- per-user override disables tool despite backend/global enablement

For host-local if pursued:

- shell command execution and timeout
- cwd/env handling
- browser bridge lifecycle
- cleanup after crashes
- OS-specific behavior

### Manual validation matrix

At minimum:

- Linux with Docker
- macOS with Docker Desktop or equivalent if supported
- existing container/microVM tiers unchanged

## Main risks

### Risk 1: Product confusion about what “process tier” means

If Option A is chosen, people may assume process tier is Docker-free when it is not. This is a UX/documentation problem and must be handled explicitly.

### Risk 2: Security overclaim

If Option B is chosen, it is easy to overstate the safety of host-local shell/browser. The documentation and UI must be precise.

### Risk 3: Browser complexity hides inside “just enable browser”

Browser is not just another boolean. It carries bridge lifecycle, shell-policy, binary/runtime dependencies, and troubleshooting complexity.

### Risk 4: Hidden config coupling

The current browser shell overlay writes to a workspace agent config file. That kind of implicit mutation becomes harder to reason about as the runtime topology gets more mixed. Refactoring this early will reduce future surprises.

## Recommendation summary

If the requirement is:

- **“Enable shell and browser in process tier with the best chance of success and the least churn”**  
  choose **Option A: host process runtime + existing sidecar container**

If the requirement is:

- **“Enable shell and browser in process tier with absolutely no container dependency”**  
  choose **Option B: pure host-local process tier**, but expect materially more work, more risk, and a weaker security posture

In both cases, keep config ownership split as follows:

- runner global config: process-tier backend decision
- agent config: global tool defaults/policy
- user config: restrictive-only user overrides
