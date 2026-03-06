; Shape Tree-sitter Highlighting Queries
; Updated for the complete grammar rewrite

; ===== Keywords =====
[
  "let"
  "var"
  "const"
  "if"
  "else"
  "then"
  "while"
  "for"
  "loop"
  "in"
  "return"
  "break"
  "continue"
  "match"
  "where"
  "when"
  "and"
  "or"
  "not"
  "from"
  "use"
  "as"
  "pub"
  "async"
  "await"
  "type"
  "trait"
  "interface"
  "enum"
  "impl"
  "extend"
  "method"
  "fn"
  "function"
  "comptime"
  "stream"
  "test"
  "it"
  "dyn"
  "typeof"
  "instanceof"
  "select"
  "order"
  "by"
  "group"
  "join"
  "on"
  "alert"
  "datasource"
  "query"
  "optimize"
  "assert"
  "expect"
  "should"
  "setup"
  "teardown"
  "config"
  "within"
  "over"
  "partition"
  "recursive"
  "module"
  "pattern"
  "find"
] @keyword

; Control flow keywords
[
  "if"
  "else"
  "then"
  "while"
  "for"
  "loop"
  "return"
  "break"
  "continue"
  "match"
] @keyword.control

; ===== Operators =====
[
  "+"
  "-"
  "*"
  "/"
  "%"
  "**"
  "=="
  "!="
  "<"
  ">"
  "<="
  ">="
  "~="
  "~<"
  "~>"
  "&&"
  "||"
  "&"
  "|"
  "^"
  "~"
  "!"
  "<<"
  ">>"
  "="
  "+="
  "-="
  "*="
  "/="
  "%="
  "**="
  "<<="
  ">>="
  "^="
  "&="
  "|="
  "?"
  ":"
  "=>"
  "->"
  "|>"
  "??"
  "!!"
  "?."
  ".."
  "..="
  "::"
  "approaching"
] @operator

; ===== Punctuation =====
[
  "("
  ")"
  "["
  "]"
  "{"
  "}"
] @punctuation.bracket

[
  ","
  ";"
  "."
  "..."
] @punctuation.delimiter

; ===== Annotations =====
(annotation "@" @attribute)
(annotation name: (annotation_name) @attribute)

; ===== Function definitions =====
(function_definition name: (identifier) @function.definition)
(foreign_function_definition name: (identifier) @function.definition)
(foreign_function_definition language: (foreign_language_identifier) @keyword)

; Function calls
(call_expression function: (identifier) @function.call)
(call_expression function: (member_expression property: (identifier) @function.method))

; ===== Type definitions =====
(struct_type_def name: (identifier) @type.definition)
(type_alias_def name: (identifier) @type.definition)
(enum_def name: (identifier) @type.definition)
(trait_def name: (identifier) @type.definition)
(interface_def name: (identifier) @type.definition)
(impl_block trait: (type_name (identifier) @type))
(impl_block type: (type_name (identifier) @type))

; Enum variants
(enum_variant_unit name: (identifier) @type.enum.variant)
(enum_variant_tuple name: (identifier) @type.enum.variant)
(enum_variant_struct name: (identifier) @type.enum.variant)

; Enum constructors
(enum_constructor_expression type: (identifier) @type)
(enum_constructor_expression variant: (identifier) @type.enum.variant)

; Type references
(basic_type) @type.builtin
(generic_type (identifier) @type)
(dyn_type (identifier) @type)
(type_name (identifier) @type)
(struct_literal type: (identifier) @type)

; ===== Variables =====
(variable_declaration pattern: (identifier) @variable)

; Parameters
(function_param pattern: (identifier) @variable.parameter)

; ===== Member access =====
(member_expression property: (identifier) @property)
(optional_member_expression property: (identifier) @property)

; ===== Object fields =====
(object_value_field key: (identifier) @property)
(object_typed_field key: (identifier) @property)
(struct_field name: (identifier) @property)

; ===== Method definitions =====
(method_def name: (identifier) @function.method)

; ===== Extend block =====
(extend_statement type: (type_name (identifier) @type))

; ===== Import / Module =====
(import_statement path: (module_path) @module)
(import_item name: (identifier) @variable)
(module_path (path_segment) @module)
(module_def name: (identifier) @module)

; ===== Test definitions =====
(test_def name: (string) @string.special)
(test_case name: (string) @string.special)

; ===== Match patterns =====
(pattern_wildcard) @variable.builtin
(pattern_identifier (identifier) @variable)
(pattern_constructor type: (identifier) @type)

; ===== Pattern block (deprecated) =====
(pattern_block name: (identifier) @function.definition)

; ===== Find statement (deprecated) =====
(find_statement pattern: (identifier) @function.call)

; ===== Stream =====
(stream_def name: (identifier) @type)

; ===== Literals =====
(number) @number
(integer) @number
(percent_literal) @number
(decimal_literal) @number

(string) @string
(simple_string) @string
(formatted_string) @string
(triple_string) @string
(formatted_triple_string) @string

(boolean) @constant.builtin.boolean
(none_literal) @constant.builtin
(self_expression) @variable.builtin

(duration) @number
(timeframe) @type

; ===== DateTime =====
(datetime_expression "@" @string.special)
(temporal_navigation) @function.builtin

; ===== Some/None =====
(some_expression "Some" @type.builtin)

; ===== Comments =====
(comment) @comment

; ===== Identifiers (catch-all, lowest priority) =====
(identifier) @variable
