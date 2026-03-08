# OSP Docs

Start here if you are using `osp`.

## Common Tasks

- Output and invocation flags: [FORMATTING.md](FORMATTING.md)
- REPL usage: [REPL.md](REPL.md)
- Config and profiles: [CONFIG.md](CONFIG.md)
- Themes and presentation: [THEMES.md](THEMES.md)
- Plugin usage: [USING_PLUGINS.md](USING_PLUGINS.md)
- Plugin authoring: [WRITING_PLUGINS.md](WRITING_PLUGINS.md)
- Troubleshooting: [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

## Other References

- DSL pipeline reference: [DSL.md](DSL.md)
- Completion and history: [COMPLETION.md](COMPLETION.md)
- Rendering and UI behavior: [UI.md](UI.md)
- Logging and debug behavior: [LOGGING.md](LOGGING.md)
- Plugin packaging: [PLUGIN_PACKAGING.md](PLUGIN_PACKAGING.md)
- Plugin protocol: [PLUGIN_PROTOCOL.md](PLUGIN_PROTOCOL.md)

## Quick Start

```bash
osp ldap user alice --json
```

```bash
osp
```

```text
ldap user alice --table -v
ldap user alice --cache | P uid mail
```
