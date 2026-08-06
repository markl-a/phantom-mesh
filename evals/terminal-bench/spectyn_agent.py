"""Terminal-Bench adapter for the spectyn-mesh autonomous agent.

spectyn is a Rust binary, so we use Terminal-Bench's *installed agent* path: the
harness copies an install script into each task container, the script downloads a
linux build of `spectyn` and writes an `agents.toml`, and the agent is driven
headlessly via `spectyn exec` (SPECTYN_AUTO_APPROVE=1) against the task terminal.

The container `agents.toml` is built here in Python (so we can assemble a
multi-provider failover chain — free tiers 429/503 a lot), base64-encoded, and
passed in via the environment; the install script just decodes it.

Run it with (see README.md for building/hosting the linux binary):

    export GROQ_API_KEY=... GEMINI_API_KEY=...
    export DOCKER_HOST=unix://$HOME/.docker/run/docker.sock   # Docker Desktop mac
    export SPECTYN_TB_BINARY_URL=http://host.docker.internal:8077/spectyn-aarch64-linux
    PYTHONPATH=. uv run tb run \
        --agent-import-path spectyn_agent:SpectynAgent \
        --model groq/llama-3.3-70b-versatile \
        --dataset-path ~/.cache/terminal-bench/terminal-bench-core/0.1.1 \
        --task-id fibonacci-server
"""

import base64
import os
import shlex

from terminal_bench.agents.installed_agents.abstract_installed_agent import (
    AbstractInstalledAgent,
)
from terminal_bench.terminal.models import TerminalCommand

# provider name (== agents.toml `type`) -> (env var holding its key, default model).
_PROVIDERS = {
    "anthropic": ("ANTHROPIC_API_KEY", "claude-sonnet-4-6"),
    "openai": ("OPENAI_API_KEY", "gpt-5.1"),
    "groq": ("GROQ_API_KEY", "llama-3.3-70b-versatile"),
    "gemini": ("GEMINI_API_KEY", "gemini-2.5-flash"),
    "cerebras": ("CEREBRAS_API_KEY", "gpt-oss-120b"),
    "openrouter": ("OPENROUTER_API_KEY", "meta-llama/llama-3.3-70b-instruct:free"),
    "mistral": ("MISTRAL_API_KEY", "mistral-small-latest"),
}

_DEFAULT_BINARY_URL = (
    "https://github.com/markl-a/spectyn-mesh/releases/latest/download/"
    "spectyn-x86_64-linux"
)

_AGENT_TOOLS = (
    '["shell", "file_read", "file_write", "file_edit", '
    '"content_search", "glob_search", "ls"]'
)
_INSTRUCTIONS = (
    "You are spectyn, an autonomous terminal agent. Use tools to actually "
    "perform the task; never just describe what to do. Keep working until the "
    "task is fully complete and verified, then stop."
)


class SpectynAgent(AbstractInstalledAgent):
    """Drives `spectyn exec` inside the task container."""

    @staticmethod
    def name() -> str:
        return "spectyn"

    def __init__(self, model_name: str = "groq/llama-3.3-70b-versatile", **kwargs):
        super().__init__(**kwargs)
        # tbench passes --model as "provider/model"; the provider becomes the
        # primary, with all other key-bearing providers appended as failover.
        if "/" in model_name:
            self._provider, self._model = model_name.split("/", 1)
        else:
            self._provider = os.environ.get("SPECTYN_TB_PROVIDER", "groq")
            self._model = model_name

        self._binary_url = os.environ.get("SPECTYN_TB_BINARY_URL", _DEFAULT_BINARY_URL)
        self._agent = os.environ.get("SPECTYN_TB_AGENT", "master")
        self._max_rounds = os.environ.get("SPECTYN_MAX_ROUNDS", "40")

    def _build_agents_toml(self) -> str:
        """Assemble the container agents.toml: every provider whose API key is
        present, primary first, with a failover chain on the master agent."""
        blocks, chain = [], []

        def add(name: str, model: str) -> None:
            spec = _PROVIDERS.get(name)
            if not spec or name in chain:
                return
            key = os.environ.get(spec[0])
            if not key:
                return
            blocks.append(
                f"[providers.{name}]\n"
                f'type = "{name}"\n'
                f'api_key = "{key}"\n'
                f'default_model = "{model}"\n'
            )
            chain.append(name)

        add(self._provider, self._model)  # primary keeps the --model choice
        for name, (_env, default_model) in _PROVIDERS.items():
            add(name, default_model)  # the rest as failover

        if not chain:
            raise RuntimeError(
                "no provider API key found in environment; set e.g. GROQ_API_KEY"
            )

        chain_list = ", ".join(f'"{c}"' for c in chain)
        return (
            f"[core]\nmax_rounds = {self._max_rounds}\ntoken_budget = 80000\n\n"
            + "\n".join(blocks)
            + "\n[agent.master]\n"
            + f'provider = "{chain[0]}"\n'
            + f"providers = [{chain_list}]\n"
            + f'instructions = "{_INSTRUCTIONS}"\n'
            + f"tools = {_AGENT_TOOLS}\n"
        )

    @property
    def _env(self) -> dict[str, str]:
        toml_b64 = base64.b64encode(self._build_agents_toml().encode()).decode()
        return {
            "SPECTYN_AUTO_APPROVE": "1",
            "SPECTYN_MAX_ROUNDS": self._max_rounds,
            "SPECTYN_TB_BINARY_URL": self._binary_url,
            "SPECTYN_TB_AGENTS_TOML_B64": toml_b64,
        }

    @property
    def _install_agent_script_path(self) -> os.PathLike:
        return self._get_templated_script_path("spectyn-setup.sh.j2")

    def _run_agent_commands(self, instruction: str) -> list[TerminalCommand]:
        escaped = shlex.quote(instruction)
        return [
            TerminalCommand(
                command=(
                    f"SPECTYN_AUTO_APPROVE=1 spectyn exec "
                    f"--agent {self._agent} --quiet {escaped}"
                ),
                min_timeout_sec=0.0,
                max_timeout_sec=float("inf"),
                block=True,
                append_enter=True,
            ),
        ]
