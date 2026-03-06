//! Language keywords metadata

use super::types::{KeywordCategory, KeywordInfo};

/// Get all language keywords
pub fn keywords() -> Vec<KeywordInfo> {
    vec![
        // Declarations
        KeywordInfo {
            keyword: "let".to_string(),
            description: "Declare a mutable variable.\n\n`let x = 42`\n`let name: string = \"hello\"`".to_string(),
            category: KeywordCategory::Declaration,
        },
        KeywordInfo {
            keyword: "var".to_string(),
            description: "Declare a mutable variable.\n\n`var count = 0`".to_string(),
            category: KeywordCategory::Declaration,
        },
        KeywordInfo {
            keyword: "const".to_string(),
            description: "Declare an immutable constant.\n\n`const PI = 3.14159`".to_string(),
            category: KeywordCategory::Declaration,
        },
        KeywordInfo {
            keyword: "fn".to_string(),
            description: "Define a named function.\n\n`fn add(x: number, y: number) -> number { return x + y }`\n\n`function` is still accepted as a legacy alias.".to_string(),
            category: KeywordCategory::Declaration,
        },
        KeywordInfo {
            keyword: "function".to_string(),
            description: "Legacy alias for `fn`.\n\n`function add(x: number, y: number) -> number { return x + y }`".to_string(),
            category: KeywordCategory::Declaration,
        },
        // Control Flow
        KeywordInfo {
            keyword: "if".to_string(),
            description: "Conditional branching.\n\n`if x > 10 { \"big\" } else { \"small\" }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "else".to_string(),
            description: "Else clause for if statement.\n\n`if cond { a } else { b }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "while".to_string(),
            description: "Loop while condition is true.\n\n`while x > 0 { x = x - 1 }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "for".to_string(),
            description: "Loop over a range or iterable.\n\n`for x in 0..10 { print(x) }`\n`for (let i = 0; i < 10; i++) { }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "return".to_string(),
            description: "Return a value from a function.\n\n`return x + y`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "break".to_string(),
            description: "Break out of a loop.\n\n`while true { if done { break } }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "continue".to_string(),
            description: "Continue to next loop iteration.\n\n`for x in items { if skip(x) { continue } }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "match".to_string(),
            description: "Pattern match on a value.\n\n`match color { Color::Red => \"red\", _ => \"other\" }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "try".to_string(),
            description: "Handle errors with try-catch.\n\n`try { risky() } catch (e) { fallback() }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        // Query Keywords
        KeywordInfo {
            keyword: "find".to_string(),
            description: "Find patterns in data.\n\n`find hammer in data[0:100]`".to_string(),
            category: KeywordCategory::Query,
        },
        KeywordInfo {
            keyword: "scan".to_string(),
            description: "Scan for conditions in data.\n\n`scan data for condition`".to_string(),
            category: KeywordCategory::Query,
        },
        KeywordInfo {
            keyword: "analyze".to_string(),
            description: "Analyze data.\n\n`analyze data with strategy`".to_string(),
            category: KeywordCategory::Query,
        },
        KeywordInfo {
            keyword: "simulate".to_string(),
            description: "Run a simulation.\n\n`simulate strategy on data`".to_string(),
            category: KeywordCategory::Query,
        },
        KeywordInfo {
            keyword: "all".to_string(),
            description: "All matches quantifier.\n\n`find all hammer in data`".to_string(),
            category: KeywordCategory::Query,
        },
        // Module System
        KeywordInfo {
            keyword: "import".to_string(),
            description:
                "Import a namespace module.\n\n`import duckdb`\n`import ml as inference`\n`use duckdb`"
                    .to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "use".to_string(),
            description: "Import named items from a module.\n\n`from std::finance use { sma, rsi }`".to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "pub".to_string(),
            description: "Make a definition publicly visible.\n\n`pub fn helper() { }`".to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "from".to_string(),
            description: "Specify module source for import.\n\n`from std::module use { name }`".to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "module".to_string(),
            description: "Reserved keyword (file = module).".to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "as".to_string(),
            description: "Alias for import/pub.\n\n`from mod use { longName as short }`".to_string(),
            category: KeywordCategory::Module,
        },
        KeywordInfo {
            keyword: "default".to_string(),
            description: "Reserved keyword.".to_string(),
            category: KeywordCategory::Module,
        },
        // Type System
        KeywordInfo {
            keyword: "type".to_string(),
            description: "Define a struct type or type alias.\n\n`type Point { x: number, y: number }`\n`type ID = string`".to_string(),
            category: KeywordCategory::Type,
        },
        KeywordInfo {
            keyword: "interface".to_string(),
            description: "Define a structural type contract.\n\n`interface Printable { format(): string }`".to_string(),
            category: KeywordCategory::Type,
        },
        KeywordInfo {
            keyword: "enum".to_string(),
            description: "Define an enum with variants.\n\n`enum Color { Red, Green, Blue(number) }`".to_string(),
            category: KeywordCategory::Type,
        },
        KeywordInfo {
            keyword: "extend".to_string(),
            description: "Add methods to an existing type.\n\n`extend Table<Row> { method count() { ... } }`".to_string(),
            category: KeywordCategory::Type,
        },
        KeywordInfo {
            keyword: "stream".to_string(),
            description: "Define a real-time data stream.\n\n`stream Feed { config { provider: \"ws\" }, on_event(e) { } }`".to_string(),
            category: KeywordCategory::Other,
        },
        // Literals
        KeywordInfo {
            keyword: "true".to_string(),
            description: "Boolean true value.".to_string(),
            category: KeywordCategory::Literal,
        },
        KeywordInfo {
            keyword: "false".to_string(),
            description: "Boolean false value.".to_string(),
            category: KeywordCategory::Literal,
        },
        KeywordInfo {
            keyword: "None".to_string(),
            description: "Empty Option value.".to_string(),
            category: KeywordCategory::Literal,
        },
        KeywordInfo {
            keyword: "Some".to_string(),
            description: "Option constructor with a value.".to_string(),
            category: KeywordCategory::Literal,
        },
        // Operators
        KeywordInfo {
            keyword: "and".to_string(),
            description: "Logical AND operator.\n\n`if a and b { }`".to_string(),
            category: KeywordCategory::Operator,
        },
        KeywordInfo {
            keyword: "or".to_string(),
            description: "Logical OR operator.\n\n`if a or b { }`".to_string(),
            category: KeywordCategory::Operator,
        },
        KeywordInfo {
            keyword: "not".to_string(),
            description: "Logical NOT operator.\n\n`if not done { }`".to_string(),
            category: KeywordCategory::Operator,
        },
        KeywordInfo {
            keyword: "in".to_string(),
            description: "Membership test operator.\n\n`if x in [1, 2, 3] { }`".to_string(),
            category: KeywordCategory::Operator,
        },
        // Temporal
        KeywordInfo {
            keyword: "on".to_string(),
            description: "Timeframe context switch.\n\n`on(1h) { sma(20) }`".to_string(),
            category: KeywordCategory::Temporal,
        },
        // Other
        // "pattern" and "strategy" are user-defined annotation names, not language keywords
        KeywordInfo {
            keyword: "method".to_string(),
            description: "Define a method on a type.\n\n`method name(params) { body }`".to_string(),
            category: KeywordCategory::Other,
        },
        KeywordInfo {
            keyword: "when".to_string(),
            description: "Conditional method clause.\n\n`when condition { action }`".to_string(),
            category: KeywordCategory::Other,
        },
        KeywordInfo {
            keyword: "this".to_string(),
            description: "Reference to current context.\n\n`this.field`".to_string(),
            category: KeywordCategory::Other,
        },
        KeywordInfo {
            keyword: "comptime".to_string(),
            description: "Declare a compile-time constant field on a struct type.\n\nComptime fields are baked at compile time and have zero runtime cost.\nThey cannot be set in struct literals.\n\n`type Currency { comptime symbol: string = \"$\", amount: number }`\n\nUse type aliases to override: `type EUR = Currency { symbol: \"\\u{20ac}\" }`".to_string(),
            category: KeywordCategory::Type,
        },
        // Async
        KeywordInfo {
            keyword: "await".to_string(),
            description: "Await an asynchronous expression or join concurrent branches.\n\n`let result = await fetch(\"url\")`\n`let (a, b) = await join all { task_a(), task_b() }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "join".to_string(),
            description: "Join concurrent branches with a strategy.\n\nUsed after `await` to run multiple async expressions concurrently.\n\n`await join all { a(), b() }`\n`await join race { fast(), slow() }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "race".to_string(),
            description: "Join strategy: return the first branch to complete, cancel the rest.\n\n`await join race { fetch(\"a\"), fetch(\"b\") }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "any".to_string(),
            description: "Join strategy: return the first branch to succeed (non-error), cancel the rest.\n\n`await join any { fetch(\"a\"), fetch(\"b\") }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "settle".to_string(),
            description: "Join strategy: wait for all branches, preserving individual success/error results.\n\n`await join settle { task_a(), task_b() }`".to_string(),
            category: KeywordCategory::ControlFlow,
        },
        KeywordInfo {
            keyword: "async".to_string(),
            description: "Mark a function as asynchronous.\n\n`async fn fetch_data(url: string) { await http.get(url) }`".to_string(),
            category: KeywordCategory::Declaration,
        },
    ]
}
