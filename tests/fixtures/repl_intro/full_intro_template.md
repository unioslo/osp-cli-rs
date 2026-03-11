## OSP
Welcome `{{display_name}}`!

```osp
[
  {"name": "Logged in as", "short_help": "{{user.name}}"},
  {"name": "Theme", "short_help": "{{theme_display}}"},
  {"name": "Version", "short_help": "{{version}}"}
]
```

## Keybindings
```osp
[
  {"name": "Ctrl-D", "short_help": "exit"},
  {"name": "Ctrl-L", "short_help": "clear screen"},
  {"name": "Ctrl-R", "short_help": "reverse search history"}
]
```

## Pipes
```osp
[
  "`F` key>3",
  "`P` col1 col2",
  "`S` sort_key",
  "`G` group_by_k1 k2",
  "`A` metric()",
  "`L` limit offset",
  "`C` count",
  "`K` key-only quick search",
  "`V` value-only quick search",
  "`contains` quick-search text",
  "`!not` negate a quick match",
  "`?exist` truthy / exists",
  "`!?not_exist` missing / falsy",
  "`= exact` exact match (ci)",
  "`== case-sens.` exact match (cs)",
  "`| H <verb>` verb help, e.g. `| H F`"
]
```

{{ help }}
