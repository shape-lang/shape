# Shape Module System

The Shape module system allows you to organize code into reusable modules that can be imported and shared across projects.

## Module Search Paths

Shape searches for modules in the following locations, in order:

### 1. Standard Library (`stdlib`)
The standard library is searched first and contains built-in modules for patterns, indicators, and utilities.

Default location:
- Workspace: `shape/shape-core/stdlib/`

Override (optional):
- Set `SHAPE_STDLIB_PATH` to use a different stdlib root.

### 2. Module Paths
User modules are searched in these default paths:
- Current directory: `.`
- Project modules: `.shape/`, `shape_modules/`, `modules/`
- User modules: `~/.shape/modules/`, `~/.local/share/shape/modules/`
- System modules: `/usr/local/share/shape/modules/`, `/usr/share/shape/modules/`

### 3. Environment Variable
Additional paths can be specified using the `SHAPE_PATH` environment variable:
```bash
export SHAPE_PATH=/path/to/modules:/another/path
```

To override the stdlib location explicitly:
```bash
export SHAPE_STDLIB_PATH=/path/to/stdlib
```

## Import Types

### Module Name Imports
```shape
import { sma, ema } from "indicators";
import * as patterns from "patterns/candlesticks";
```
These search in all configured module paths.

### Relative Imports
```shape
import { helper } from "./utils";
import { shared } from "../common/shared";
```
These are resolved relative to the current file.

### Absolute Imports
```shape
import { config } from "/etc/shape/config";
```
These use absolute filesystem paths.

## Module Resolution

1. If the import path starts with `./` or `../`, it's treated as a relative import
2. If it starts with `/`, it's treated as an absolute path
3. Otherwise, it's searched in the module paths
4. If no extension is provided, `.shape` is automatically added
5. If a directory is specified, Shape looks for `index.shape` within it

## Creating Modules

### Basic Module
```shape
// math.shape
export function add(a, b) {
    return a + b;
}

export function multiply(a, b) {
    return a * b;
}
```

### Module with Patterns
```shape
// patterns/reversal.shape
export pattern hammer {
    body = abs(close - open);
    range = high - low;
    body < range * 0.3 and
    lower_shadow > body * 2
}

export pattern shooting_star {
    body = abs(close - open);
    range = high - low;
    body < range * 0.3 and
    upper_shadow > body * 2
}
```

### Named Exports
```shape
// utils.shape
function internalHelper() {
    // Not exported
    return 42;
}

export function publicHelper() {
    return internalHelper() * 2;
}

export { publicHelper as helper };
```

## Best Practices

1. **Organization**: Group related functionality into modules
2. **Naming**: Use descriptive module names that indicate their purpose
3. **Exports**: Only export what's needed by other modules
4. **Dependencies**: Avoid circular dependencies between modules
5. **Documentation**: Include comments explaining what each module provides

## Example Project Structure

```
my-trading-project/
├── .shape/
│   └── config.shape
├── modules/
│   ├── strategies/
│   │   ├── index.shape
│   │   ├── trend_following.shape
│   │   └── mean_reversion.shape
│   ├── indicators/
│   │   ├── custom_rsi.shape
│   │   └── pivot_points.shape
│   └── utils/
│       ├── math.shape
│       └── formatting.shape
└── main.shape
```

## Debugging Module Loading

If a module cannot be found, Shape will show which paths were searched:

```
Module not found: mymodule
Searched in:
  stdlib: /path/to/shape/shape-core/stdlib
  .
  .shape
  shape_modules
  modules
  /home/user/.shape/modules
  /home/user/.local/share/shape/modules
```
