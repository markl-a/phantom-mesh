Loaded cached credentials.
Hook registry initialized with 0 hook entries
I will check `docs/badges.md` for existing badge configurations.
I will list the `docs` directory to verify the file's existence.
I will read `docs/badges.md` to see the existing badge definitions.
I will search for badge patterns in `docs/badges.md` using `grep_search`.
I will write the Python script to generate the requested badges.
Error executing tool write_file: Tool "write_file" not found. Did you mean one of: "read_file", "cli_help", "activate_skill"?
I will use `cli_help` to check for the availability of `write_file` and `run_shell_command`.
I will try to use `run_shell_command` to list files and confirm its availability.
Error executing tool run_shell_command: Tool "run_shell_command" not found. Did you mean one of: "grep_search", "cli_help", "read_file"?
```python
def generate_badges():
    badges = [
        ("Python 3.11+", "https://img.shields.io/badge/python-3.11+-blue.svg"),
        ("License", "https://img.shields.io/badge/license-Apache%202.0-blue.svg"),
        ("Tests", "https://img.shields.io/badge/tests-passing-brightgreen.svg"),
        ("Ruff", "https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json"),
        ("Platforms", "https://img.shields.io/badge/platform-windows%20%7C%20macos%20%7C%20linux-lightgrey.svg"),
        ("Requirement", "https://img.shields.io/badge/requirement-Ollama-orange.svg"),
    ]
    
    for label, url in badges:
        print(f"![{label}]({url})")

if __name__ == "__main__":
    generate_badges()
```

![Python 3.11+](https://img.shields.io/badge/python-3.11+-blue.svg)
![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)
![Tests](https://img.shields.io/badge/tests-passing-brightgreen.svg)
![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)
![Platforms](https://img.shields.io/badge/platform-windows%20%7C%20macos%20%7C%20linux-lightgrey.svg)
![Requirement](https://img.shields.io/badge/requirement-Ollama-orange.svg)
