# Installation and setup - 0.10.0

First resolve code-rcl on PATH (`Get-Command code-rcl` on Windows,
`command -v code-rcl` on Unix). Check ~/.code-rcl/bin if PATH is stale.
Use the resolved absolute path and check --version.

If the binary is absent, installation is separate from setup. Use the project's
published release installer, or `cargo install codebase-recall --version 0.10.0`
after that release exists. For development use `cargo build` / `cargo test`
from the checkout, not an older installed release.

`code-rcl setup --workspace` installs the complete skill bundle and configures
a workspace MCP root. `--skill-only`, `--mcp-only`, and `--skill-dir` restrict
what is installed. Global configuration requires explicit `setup --global`;
MCP calls then need project unless the server was launched with --project.

Setup does not install the executable. It cannot run before the binary exists.
Do not replace unrelated user configuration. Review generated paths and use
explicit install scope when configuring an agent integration.

For semantic/hybrid retrieval and explicit model/index setup in 0.10.0, read
[embeddings.md](embeddings.md). Source and embedding freshness are separate.

Setup supports Codex, Claude Code, and Antigravity through the same command.
Use `--target codex|claude|antigravity|gemini|all`; default all. Workspace is the
default scope, global is explicit. Codex MCP is TOML and project config requires
a trusted workspace; setup does not alter trust.

Remove integrations with `code-rcl uninstall --workspace --target codex` or
`code-rcl uninstall --global --target all`. Component selectors and custom
`--skill-dir` mirror setup. Shared resources remain while another target owns
them; single-target legacy removal conservatively retains unknown shared
ownership. User-added files, other settings, binary, model and cache remain.
Legacy `setup --remove` removes only optional instruction blocks/hooks.
