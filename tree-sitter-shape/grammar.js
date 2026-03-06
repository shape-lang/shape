/// Tree-sitter grammar for the Shape language
/// Translated from shape/shape-ast/src/shape.pest (authoritative grammar)

// Precedence levels (numeric, higher = tighter binding)
const PREC = {
  ASSIGNMENT: 1,
  PIPE: 2,
  TERNARY: 3,
  NULL_COALESCE: 4,
  CONTEXT: 5,
  OR: 6,
  AND: 7,
  BITWISE_OR: 8,
  BITWISE_XOR: 9,
  BITWISE_AND: 10,
  COMPARISON: 11,
  RANGE: 12,
  ADDITIVE: 13,
  SHIFT: 14,
  MULTIPLICATIVE: 15,
  EXPONENTIAL: 16,
  UNARY: 17,
  POSTFIX: 18,
  TYPE_ASSERTION: 18,
};

module.exports = grammar({
  name: 'shape',

  word: $ => $.identifier,

  extras: $ => [
    /\s/,
    $.comment,
  ],

  conflicts: $ => [
    // Identifier can be pattern, expression, or type in various contexts
    [$._destructure_pattern, $._primary_expression],
    [$._destructure_pattern, $.pattern_identifier],
    [$._destructure_pattern, $.decomposition_binding],
    // [$._destructure_pattern, $.pattern_constructor],  // removed: word token resolves this
    [$._destructure_pattern, $.basic_type],
    [$._destructure_pattern, $.basic_type, $._primary_expression],
    [$._destructure_pattern, $.type_param],
    [$._destructure_pattern, $.decomposition_binding, $.type_param],
    // Braces/brackets can be patterns, types, or expressions
    [$.destructure_object_pattern, $.block_expression, $.object_literal],
    [$.destructure_object_pattern, $.pattern_object],
    [$.destructure_object_field, $.pattern_field],
    [$.destructure_object_pattern, $.object_type],
    [$.destructure_object_pattern, $.object_type, $.object_literal, $.block_expression],
    [$.destructure_array_pattern, $.pattern_array],
    [$.destructure_array_pattern, $.array_literal],
    [$.destructure_object_field, $._primary_expression],
    [$.destructure_rest_pattern, $._primary_expression],
    // Object/block ambiguity
    [$.object_literal, $.block_expression],
    [$.object_type, $.block_expression, $.object_literal],
    [$.object_type_member, $._primary_expression],
    // Type parsing ambiguities
    [$.basic_type, $.generic_type],
    [$.basic_type, $._primary_expression],
    [$.basic_type, $.type_param],
    [$.basic_type, $.type_param, $._primary_expression],
    [$.generic_type, $._primary_expression],
    [$._base_type, $.type_param],
    [$.union_type],
    [$.intersection_type],
    [$.optional_type],
    [$.primary_type],
    [$.dyn_type],
    [$.decomposition_binding, $.type_param],
    // Keyword-initiated ambiguities
    [$.type_alias_def],
    [$.datetime_expression],
    [$.datetime_literal],
    [$.annotation],
    [$.enum_constructor_expression],
    [$.variable_declaration],
    [$.while_loop, $.block_expression],
    // Expression context ambiguities
    [$.pipe_lambda, $.type_assertion_expression],
    // if-statement with else vs if-block-expression
    [$.else_clause, $.if_block_expression],
    // struct literal vs plain expression (needed for match/if/while where { starts body)
    [$._primary_expression, $.struct_literal],
  ],

  rules: {
    // ========================================================
    // Entry Point
    // ========================================================
    program: $ => repeat($._item),

    _item: $ => choice(
      $.import_statement,
      $.pub_item,
      $.module_def,
      $.struct_type_def,
      $.type_alias_def,
      $.trait_def,
      $.interface_def,
      $.enum_def,
      $.impl_block,
      $.extend_statement,
      $.optimize_statement,
      $.annotation_def,
      $.datasource_def,
      $.query_decl,
      prec(2, $.foreign_function_definition),
      prec(1, $.function_definition),
      $.stream_def,
      $.test_def,
      $.pattern_block,
      $.query,
      $._statement,
    ),

    module_def: $ => seq(
      'module', field('name', $.identifier),
      '{', repeat($._item), '}',
    ),

    // ========================================================
    // Comments
    // ========================================================
    comment: $ => token(choice(
      seq('//', /[^\n]*/),
      seq('#', /[^\n]*/),
      seq('/*', /[^*]*\*+([^/*][^*]*\*+)*/, '/'),
    )),

    // ========================================================
    // Module System
    // ========================================================
    module_path: $ => sep1($.path_segment, '.'),

    path_segment: $ => /[a-zA-Z0-9_-]+/,

    import_statement: $ => choice(
      seq('from', field('path', choice($.module_path, $.string)), 'use', '{', $.import_item_list, '}', optional(';')),
      seq('use', field('path', $.module_path), 'as', field('alias', $.identifier), optional(';')),
      seq('use', field('path', $.module_path), optional(';')),
    ),

    import_item_list: $ => seq(
      commaSep1($.import_item),
      optional(','),
    ),

    import_item: $ => seq(
      field('name', $.identifier),
      optional(seq('as', field('alias', $.identifier))),
    ),

    pub_item: $ => seq('pub', choice(
      field('item', $.function_definition),
      field('item', $.variable_declaration),
      field('item', $.type_alias_def),
      field('item', $.enum_def),
      field('item', $.struct_type_def),
      field('item', $.trait_def),
      field('item', $.interface_def),
      seq('{', $.export_spec_list, '}', optional(';')),
    )),

    export_spec_list: $ => seq(commaSep1($.export_spec), optional(',')),

    export_spec: $ => seq(
      field('name', $.identifier),
      optional(seq('as', field('alias', $.identifier))),
    ),

    // ========================================================
    // Type Definitions
    // ========================================================
    type_alias_def: $ => seq(
      'type', field('name', $.identifier),
      optional($.type_params), '=',
      field('type', $.type_annotation),
      optional($.comptime_field_overrides),
      optional(';'),
    ),

    comptime_field_overrides: $ => seq(
      '{', commaSep1($.comptime_field_override), optional(','), '}',
    ),

    comptime_field_override: $ => seq(
      field('name', $.identifier), ':', field('value', $._expression),
    ),

    struct_type_def: $ => seq(
      'type', field('name', $.identifier),
      optional($.type_params),
      '{', optional($.struct_field_list), '}',
    ),

    struct_field_list: $ => seq(commaSep1($.struct_field), optional(',')),

    struct_field: $ => seq(
      optional($.annotations),
      optional('comptime'),
      field('name', $.identifier), ':',
      field('type', $.type_annotation),
      optional(seq('=', field('default', $._expression))),
    ),

    // ========================================================
    // Comptime
    // ========================================================
    comptime_block: $ => seq('comptime', $.block_expression),

    comptime_for_expression: $ => seq(
      'comptime', 'for',
      field('variable', $.identifier), 'in',
      field('target', $._primary_expression),
      '{', repeat($._statement), '}',
    ),

    // ========================================================
    // Annotated Expression
    // ========================================================
    // annotated_expression: @annotation expr
    // Note: does NOT match when followed by fn/function (that's function_definition)
    annotated_expression: $ => prec(PREC.UNARY, seq(
      repeat1($.annotation), $._postfix_expression,
    )),

    // ========================================================
    // Interface Definitions
    // ========================================================
    interface_def: $ => seq(
      'interface', field('name', $.identifier),
      optional($.type_params),
      optional($.extends_clause),
      '{', optional($.interface_member_list), '}',
    ),

    type_params: $ => seq('<', commaSep1($.type_param_name), '>'),

    type_param_name: $ => seq(
      field('name', $.identifier),
      optional(seq('extends', field('bound', $.type_annotation))),
      optional(seq(':', $.trait_bound_list)),
    ),

    trait_bound_list: $ => sep1($.identifier, '+'),

    extends_clause: $ => seq('extends', commaSep1($.type_annotation)),

    interface_member_list: $ => seq(
      $.interface_member,
      repeat(seq(choice(';', ','), $.interface_member)),
      optional(choice(';', ',')),
    ),

    interface_member: $ => choice(
      // Method signature (must try before property)
      seq(field('name', $.identifier), optional('?'), '(', optional($.type_param_list), ')', ':', field('type', $.type_annotation)),
      // Property signature
      seq(field('name', $.identifier), optional('?'), ':', field('type', $.type_annotation)),
      // Index signature
      seq('[', field('key', $.identifier), ':', choice('string', 'number'), ']', ':', field('type', $.type_annotation)),
    ),

    // ========================================================
    // Trait Definitions
    // ========================================================
    trait_def: $ => seq(
      'trait', field('name', $.identifier),
      optional($.type_params),
      optional($.extends_clause),
      '{', repeat($.trait_member), '}',
    ),

    trait_member: $ => choice(
      $.associated_type_decl,
      $.method_def,
      seq($.interface_member, optional(choice(';', ','))),
    ),

    associated_type_decl: $ => seq(
      'type', field('name', $.identifier),
      optional(seq(':', $.trait_bound_list)), ';',
    ),

    // ========================================================
    // Impl Blocks
    // ========================================================
    impl_block: $ => seq(
      'impl', field('trait', $.type_name), 'for', field('type', $.type_name),
      optional($.where_clause),
      '{', repeat($.impl_member), '}',
    ),

    impl_member: $ => choice($.associated_type_binding, $.method_def),

    associated_type_binding: $ => seq(
      'type', field('name', $.identifier), '=', field('type', $.type_annotation), ';',
    ),

    // ========================================================
    // Enum Definitions
    // ========================================================
    enum_def: $ => seq(
      'enum', field('name', $.identifier),
      optional($.type_params),
      '{', optional($.enum_member_list), '}',
    ),

    enum_member_list: $ => seq(
      $.enum_member,
      repeat(seq(choice(';', ','), $.enum_member)),
      optional(choice(';', ',')),
    ),

    enum_member: $ => choice(
      $.enum_variant_struct,
      $.enum_variant_tuple,
      $.enum_variant_unit,
    ),

    enum_variant_unit: $ => seq(
      field('name', $.identifier),
      optional(seq('=', choice($.string, $.number))),
    ),

    enum_variant_tuple: $ => seq(
      field('name', $.identifier),
      '(', commaSep1($.type_annotation), ')',
    ),

    enum_variant_struct: $ => seq(
      field('name', $.identifier),
      '{', optional($.object_type_member_list), '}',
    ),

    // ========================================================
    // Type Extension & Methods
    // ========================================================
    extend_statement: $ => seq(
      'extend', field('type', $.type_name),
      '{', repeat($.method_def), '}',
    ),

    type_name: $ => seq($.identifier, optional(seq('<', $.type_annotation, '>'))),

    method_def: $ => seq(
      'method', field('name', $.identifier),
      '(', optional($.function_params), ')',
      optional($.when_clause),
      optional($.return_type),
      '{', repeat($._statement), '}',
    ),

    when_clause: $ => seq('when', $._expression),

    // ========================================================
    // Function Definitions
    // ========================================================
    foreign_function_definition: $ => prec.dynamic(11, seq(
      optional($.annotations),
      optional('async'),
      choice('function', 'fn'),
      field('language', $.foreign_language_identifier),
      field('name', $.identifier),
      optional($.type_params),
      '(', optional($.function_params), ')',
      optional($.return_type),
      '{', optional(field('body', $.foreign_body)), '}',
    )),

    foreign_language_identifier: $ => alias($.identifier, $.foreign_language_identifier),

    // Raw foreign-language source kept opaque for Shape parsing.
    // Supports nested braces and quoted strings to avoid premature termination.
    foreign_body: $ => repeat1($._foreign_body_item),

    _foreign_body_item: $ => choice(
      $.foreign_brace_group,
      $.foreign_string_literal,
      $.foreign_text,
    ),

    foreign_brace_group: $ => seq('{', repeat($._foreign_body_item), '}'),

    foreign_string_literal: $ => token(choice(
      seq('"', repeat(choice(/[^"\\]+/, /\\./)), '"'),
      seq("'", repeat(choice(/[^'\\]+/, /\\./)), "'"),
    )),

    foreign_text: $ => token(/[^\{\}"']+/),

    function_definition: $ => prec.dynamic(10, seq(
      optional($.annotations),
      optional('async'),
      choice('function', 'fn'),
      field('name', $.identifier),
      optional($.type_params),
      '(', optional($.function_params), ')',
      optional($.return_type),
      optional($.where_clause),
      '{', repeat($._statement), '}',
    )),

    where_clause: $ => seq('where', commaSep1($.where_predicate), optional(',')),

    where_predicate: $ => seq($.identifier, ':', $.trait_bound_list),

    annotations: $ => repeat1($.annotation),

    annotation: $ => seq(
      '@', field('name', $.annotation_name),
      optional(seq('(', optional($.annotation_args), ')')),
    ),

    annotation_name: $ => $.identifier,

    annotation_args: $ => commaSep1($._expression),

    // ========================================================
    // Annotation Definitions
    // ========================================================
    annotation_def: $ => seq(
      'annotation', field('name', $.identifier),
      '(', optional($.annotation_def_params), ')',
      '{', repeat($.annotation_handler), '}',
    ),

    annotation_def_params: $ => commaSep1($.identifier),

    annotation_handler: $ => seq(
      field('name', choice('on_define', 'before', 'after', 'metadata', 'comptime')),
      '(', optional(commaSep1($.identifier)), ')',
      optional($.return_type),
      $.block_expression,
    ),

    // ========================================================
    // Function Parameters
    // ========================================================
    function_params: $ => commaSep1($.function_param),

    function_param: $ => seq(
      field('pattern', $._destructure_pattern),
      optional(seq(':', field('type', $.type_annotation))),
      optional(seq('=', field('default', $._expression))),
    ),

    return_type: $ => seq('->', $.type_annotation),

    // ========================================================
    // Function Expressions (lambdas, closures, arrow functions)
    // ========================================================
    function_expression: $ => choice(
      $.pipe_lambda,
      $.arrow_function,
    ),

    pipe_lambda: $ => prec(PREC.POSTFIX, seq(
      '|', optional($.function_params), '|',
      choice($._expression, seq('{', repeat($._statement), '}')),
    )),

    arrow_function: $ => prec.right(PREC.ASSIGNMENT, choice(
      seq($.identifier, '=>', choice($._expression, seq('{', repeat($._statement), '}'))),
      seq('(', optional($.function_params), ')', optional($.return_type), '=>', choice($._expression, seq('{', repeat($._statement), '}'))),
    )),

    // ========================================================
    // Statements
    // ========================================================
    _statement: $ => choice(
      $.return_statement,
      $.break_statement,
      $.continue_statement,
      $.variable_declaration,
      $.assignment_statement,
      $.if_statement,
      $.for_loop,
      $.while_loop,
      $.find_statement,
      $.expression_statement,
    ),

    return_statement: $ => prec.right(seq('return', optional($._expression), optional(';'))),
    break_statement: $ => seq('break', optional(';')),
    continue_statement: $ => seq('continue', optional(';')),

    variable_declaration: $ => prec(1, seq(
      field('keyword', choice('let', 'var', 'const')),
      field('pattern', $._destructure_pattern),
      optional(seq(':', field('type', $.type_annotation))),
      optional(seq('=', field('value', $._expression))),
      optional(';'),
    )),

    assignment_statement: $ => prec(1, seq(
      field('left', $._destructure_pattern),
      '=',
      field('right', $._expression),
      optional(';'),
    )),

    expression_statement: $ => prec(-1, seq($._expression, optional(';'))),

    if_statement: $ => prec.right(prec.dynamic(10, seq(
      'if', field('condition', $._expression),
      '{', repeat($._statement), '}',
      optional($.else_clause),
    ))),

    else_clause: $ => prec.right(seq('else', choice(
      $.if_statement,
      seq('{', repeat($._statement), '}'),
    ))),

    for_loop: $ => seq(
      'for', $.for_clause,
      '{', repeat($._statement), '}',
    ),

    for_clause: $ => choice(
      seq($._destructure_pattern, 'in', $._expression),
      seq($.variable_declaration, ';', $._expression, ';', $._expression),
      seq('(', $.variable_declaration, ';', $._expression, ';', $._expression, ')'),
    ),

    while_loop: $ => prec.dynamic(10, seq('while', field('condition', $._expression), '{', repeat($._statement), '}')),

    // ========================================================
    // Stream Definitions
    // ========================================================
    // Pattern block (deprecated — use @pattern decorator instead)
    pattern_block: $ => seq(
      'pattern', field('name', $.identifier),
      optional(field('tolerance', seq('~', $.number))),
      '{', repeat($._statement), '}',
    ),

    // Find statement (deprecated — use method chaining instead)
    find_statement: $ => prec.right(seq(
      'find', field('pattern', $.identifier),
      repeat($.find_modifier),
    )),

    find_modifier: $ => prec.right(choice(
      seq('where', $._expression),
      seq('in', choice(
        seq('last', '(', $._expression, optional($.identifier), ')'),
        seq('all', optional($.identifier)),
        $._expression,
      )),
      seq('last', '(', $._expression, optional($.identifier), ')'),
      seq('on', '(', $._expression, ')'),
      seq('all', optional($.identifier)),
    )),

    stream_def: $ => seq(
      'stream', field('name', $.identifier), '{',
      optional($.stream_config),
      optional($.stream_state),
      optional(seq('on_connect', '(', ')', '{', repeat($._statement), '}')),
      optional(seq('on_disconnect', '(', ')', '{', repeat($._statement), '}')),
      optional(seq('on_event', '(', $.identifier, ')', '{', repeat($._statement), '}')),
      optional(seq('on_window', '(', $.identifier, ',', $.identifier, ')', '{', repeat($._statement), '}')),
      optional(seq('on_error', '(', $.identifier, ')', '{', repeat($._statement), '}')),
      '}',
    ),

    stream_config: $ => seq('config', '{', repeat($.stream_config_item), '}'),

    stream_config_item: $ => seq(
      field('key', $.identifier), ':', field('value', $._expression), optional(';'),
    ),

    stream_state: $ => seq('state', '{', optional(seq(
      $.variable_declaration,
      repeat(seq(';', $.variable_declaration)),
      optional(';'),
    )), '}'),

    // ========================================================
    // Data Source / Query Declarations
    // ========================================================
    datasource_def: $ => seq(
      'datasource', field('name', $.identifier), ':',
      field('type', $.type_annotation), '=',
      field('value', $._expression), optional(';'),
    ),

    query_decl: $ => seq(
      'query', field('name', $.identifier), ':',
      field('type', $.type_annotation), '=',
      field('value', $._expression), optional(';'),
    ),

    // ========================================================
    // Test Definitions
    // ========================================================
    test_def: $ => seq(
      'test', field('name', $.string), '{',
      optional(seq('setup', '{', repeat($._statement), '}')),
      optional(seq('teardown', '{', repeat($._statement), '}')),
      repeat1($.test_case),
      '}',
    ),

    test_case: $ => seq(
      choice('test', 'it'), field('name', $.string),
      optional(seq('->', $.test_tags)),
      '{', repeat(choice($.test_statement, $._statement)), '}',
    ),

    test_tags: $ => seq('[', commaSep1(choice($.identifier, $.string)), optional(','), ']'),

    test_statement: $ => choice(
      $.assert_statement,
      $.expect_statement,
      $.should_statement,
      $.test_fixture_statement,
    ),

    assert_statement: $ => seq('assert', $._expression, optional(seq(',', $.string)), optional(';')),

    expect_statement: $ => seq(
      'expect', '(', $._expression, ')', '.', $.expectation_matcher,
    ),

    expectation_matcher: $ => choice(
      seq(choice('to_be', 'toBe'), '(', $._expression, ')'),
      seq(choice('to_equal', 'toEqual'), '(', $._expression, ')'),
      seq(choice('to_be_close_to', 'toBeCloseTo'), '(', $._expression, optional(seq(',', $.number)), ')'),
      seq(choice('to_be_greater_than', 'toBeGreaterThan'), '(', $._expression, ')'),
      seq(choice('to_be_less_than', 'toBeLessThan'), '(', $._expression, ')'),
      seq(choice('to_contain', 'toContain'), '(', $._expression, ')'),
      seq(choice('to_be_truthy', 'toBeTruthy'), '(', ')'),
      seq(choice('to_be_falsy', 'toBeFalsy'), '(', ')'),
      seq(choice('to_throw', 'toThrow'), '(', optional($.string), ')'),
      seq(choice('to_match_pattern', 'toMatchPattern'), '(', $.identifier, optional(seq(',', $.test_match_options)), ')'),
    ),

    test_match_options: $ => seq('{', commaSep1($.test_match_option), optional(','), '}'),

    test_match_option: $ => choice(
      seq('fuzzy', ':', $.number),
      seq('timeframe', ':', $.timeframe),
      seq('symbol', ':', $.string),
    ),

    should_statement: $ => seq($._expression, 'should', $.should_matcher),

    should_matcher: $ => choice(
      seq('be', $._expression),
      seq('equal', $._expression),
      seq('contain', $._expression),
      seq('match', $.identifier),
      seq('be_close_to', $._expression, optional(seq('within', $.number))),
    ),

    test_fixture_statement: $ => choice(
      seq('with_data', '(', $._expression, ')', '{', repeat($._statement), '}'),
      seq('with_mock', '(', $.identifier, optional(seq(',', $._expression)), ')', '{', repeat($._statement), '}'),
    ),

    // ========================================================
    // Optimize Statement
    // ========================================================
    optimize_statement: $ => seq(
      'optimize', field('param', $.identifier), 'in',
      '[', $._expression, '..', $._expression, ']',
      'for', choice('sharpe', 'sortino', 'return', 'drawdown', 'win_rate', 'profit_factor', $._expression),
    ),

    // ========================================================
    // Queries (alert, with/CTE)
    // ========================================================
    query: $ => choice($.with_query, $.alert_query),

    with_query: $ => seq('with', commaSep1($.cte_def), $.alert_query),

    cte_def: $ => seq(
      optional('recursive'),
      field('name', $.identifier),
      optional(seq('(', commaSep1($.identifier), ')')),
      'as', '(', $.alert_query, ')',
    ),

    alert_query: $ => seq('alert', 'when', $._expression, optional($.alert_options)),
    alert_options: $ => seq('message', $.string, optional(seq('webhook', $.string))),

    // ========================================================
    // Window Functions
    // ========================================================
    // Window function is a postfix modifier: expr(...) over (...)
    window_function_call: $ => prec(PREC.POSTFIX, seq(
      field('call', $.call_expression),
      'over', '(', optional($.window_spec), ')',
    )),

    window_spec: $ => choice(
      seq($.partition_by_clause, optional($.order_by_clause), optional($.window_frame_clause)),
      seq($.order_by_clause, optional($.window_frame_clause)),
      $.window_frame_clause,
    ),

    partition_by_clause: $ => seq('partition', 'by', commaSep1($._expression)),

    order_by_clause: $ => seq('order', 'by', commaSep1($.order_by_item)),

    order_by_item: $ => seq($._expression, optional(choice('asc', 'desc', 'ASC', 'DESC'))),

    window_frame_clause: $ => seq(choice('rows', 'range'), $.frame_extent),

    frame_extent: $ => choice(
      seq('between', $.frame_bound, 'and', $.frame_bound),
      $.frame_bound,
    ),

    frame_bound: $ => choice(
      seq('unbounded', 'preceding'),
      seq('unbounded', 'following'),
      seq('current', 'row'),
      seq($.integer, 'preceding'),
      seq($.integer, 'following'),
    ),

    // ========================================================
    // JOIN Clauses
    // ========================================================
    join_clause: $ => seq(
      optional(choice(
        'inner',
        seq('left', optional('outer')),
        seq('right', optional('outer')),
        seq('full', optional('outer')),
        'cross',
      )),
      'join', $.join_source,
      optional(choice(
        seq('on', $._expression),
        seq('using', '(', commaSep1($.identifier), ')'),
        seq('within', $.duration),
      )),
    ),

    join_source: $ => choice(
      seq($.identifier, optional(seq('as', $.identifier))),
      seq('(', $.alert_query, ')', optional(seq('as', $.identifier))),
    ),

    // ========================================================
    // Destructure Patterns
    // ========================================================
    _destructure_pattern: $ => choice(
      $.destructure_decomposition_pattern,
      $.destructure_rest_pattern,
      $.destructure_array_pattern,
      $.destructure_object_pattern,
      $.identifier,
    ),

    destructure_array_pattern: $ => seq('[', optional(seq(commaSep1($._destructure_pattern), optional(','))), ']'),

    destructure_object_pattern: $ => seq('{', optional(seq(commaSep1($.destructure_object_field), optional(','))), '}'),

    destructure_object_field: $ => choice(
      seq(field('key', $.identifier), optional(seq(':', $._destructure_pattern))),
      seq('...', field('rest', $.identifier)),
    ),

    destructure_rest_pattern: $ => seq('...', $.identifier),

    destructure_decomposition_pattern: $ => seq(
      '(', $.decomposition_binding, repeat1(seq(',', $.decomposition_binding)), optional(','), ')',
    ),

    decomposition_binding: $ => seq(
      field('name', $.identifier), ':',
      choice($.type_annotation, seq('{', commaSep1($.identifier), optional(','), '}')),
    ),

    // ========================================================
    // Type Annotations
    // ========================================================
    type_annotation: $ => $.union_type,

    union_type: $ => sep1($.intersection_type, '|'),

    intersection_type: $ => sep1($.optional_type, '+'),

    optional_type: $ => seq($.primary_type, optional('?')),

    primary_type: $ => seq($._base_type, repeat(seq('[', ']'))),

    _base_type: $ => choice(
      $.tuple_type,
      $.object_type,
      $.function_type,
      $.dyn_type,
      $.generic_type,
      $.basic_type,
      seq('Array', '<', $.type_annotation, '>'),
      seq('(', $.type_annotation, ')'),
    ),

    dyn_type: $ => seq('dyn', sep1($.identifier, '+')),

    basic_type: $ => choice(
      'number', 'string', 'bool', 'boolean', 'void',
      'option', 'timestamp', 'undefined', 'any', 'never', 'pattern',
      $.identifier,
    ),

    tuple_type: $ => seq('[', $.type_annotation, repeat1(seq(',', $.type_annotation)), ']'),

    object_type: $ => seq('{', optional($.object_type_member_list), '}'),

    object_type_member_list: $ => seq(
      $.object_type_member,
      repeat(seq(choice(';', ','), $.object_type_member)),
      optional(choice(';', ',')),
    ),

    object_type_member: $ => seq(
      field('name', $.identifier), optional('?'), ':', field('type', $.type_annotation),
    ),

    function_type: $ => seq('(', optional($.type_param_list), ')', '=>', $.type_annotation),

    type_param_list: $ => commaSep1($.type_param),

    type_param: $ => choice(
      seq($.identifier, optional('?'), ':', $.type_annotation),
      $.type_annotation,
    ),

    generic_type: $ => seq($.identifier, '<', commaSep1($.type_annotation), '>'),

    // ========================================================
    // Expressions
    // ========================================================
    _expression: $ => choice(
      $.assignment_expression,
      $.compound_assignment_expression,
      $.pipe_expression,
      $.ternary_expression,
      $.null_coalesce_expression,
      $.context_expression,
      $.or_expression,
      $.and_expression,
      $.bitwise_or_expression,
      $.bitwise_xor_expression,
      $.bitwise_and_expression,
      $.comparison_expression,
      $.fuzzy_comparison_expression,
      $.instanceof_expression,
      $.range_expression,
      $.additive_expression,
      $.shift_expression,
      $.multiplicative_expression,
      $.exponential_expression,
      $.unary_expression,
      $.type_assertion_expression,
      $._postfix_expression,
    ),


    assignment_expression: $ => prec.right(PREC.ASSIGNMENT, seq(
      field('left', $._postfix_expression), '=', field('right', $._expression),
    )),

    compound_assignment_expression: $ => prec.right(PREC.ASSIGNMENT, seq(
      field('left', $._postfix_expression),
      field('operator', choice('+=', '-=', '*=', '/=', '%=', '**=', '<<=', '>>=', '^=', '&=', '|=')),
      field('right', $._expression),
    )),

    pipe_expression: $ => prec.left(PREC.PIPE, seq(
      field('left', $._expression), '|>', field('right', $._expression),
    )),

    ternary_expression: $ => prec.right(PREC.TERNARY, seq(
      field('condition', $._expression), '?',
      field('consequence', $._expression), ':',
      field('alternative', $._expression),
    )),

    null_coalesce_expression: $ => prec.left(PREC.NULL_COALESCE, seq(
      field('left', $._expression), '??', field('right', $._expression),
    )),

    context_expression: $ => prec.left(PREC.CONTEXT, seq(
      field('left', $._expression), '!!', field('right', $._expression),
    )),

    or_expression: $ => prec.left(PREC.OR, seq(
      field('left', $._expression),
      field('operator', choice('||', 'or')),
      field('right', $._expression),
    )),

    and_expression: $ => prec.left(PREC.AND, seq(
      field('left', $._expression),
      field('operator', choice('&&', 'and')),
      field('right', $._expression),
    )),

    bitwise_or_expression: $ => prec.left(PREC.BITWISE_OR, seq(
      field('left', $._expression), '|', field('right', $._expression),
    )),

    bitwise_xor_expression: $ => prec.left(PREC.BITWISE_XOR, seq(
      field('left', $._expression), '^', field('right', $._expression),
    )),

    bitwise_and_expression: $ => prec.left(PREC.BITWISE_AND, seq(
      field('left', $._expression), '&', field('right', $._expression),
    )),

    comparison_expression: $ => prec.left(PREC.COMPARISON, seq(
      field('left', $._expression),
      field('operator', choice('>=', '<=', '==', '!=', '>', '<', 'approaching')),
      field('right', $._expression),
    )),

    fuzzy_comparison_expression: $ => prec.left(PREC.COMPARISON, seq(
      field('left', $._expression),
      field('operator', choice('~=', '~<', '~>')),
      field('right', $._expression),
      optional(seq('within', $.number, optional('%'))),
    )),

    instanceof_expression: $ => prec.left(PREC.COMPARISON, seq(
      field('left', $._expression), 'instanceof', field('type', $.type_annotation),
    )),

    range_expression: $ => prec.left(PREC.RANGE, seq(
      field('start', $._expression),
      field('operator', choice('..=', '..')),
      field('end', $._expression),
    )),

    additive_expression: $ => prec.left(PREC.ADDITIVE, seq(
      field('left', $._expression),
      field('operator', choice('+', '-')),
      field('right', $._expression),
    )),

    shift_expression: $ => prec.left(PREC.SHIFT, seq(
      field('left', $._expression),
      field('operator', choice('<<', '>>')),
      field('right', $._expression),
    )),

    multiplicative_expression: $ => prec.left(PREC.MULTIPLICATIVE, seq(
      field('left', $._expression),
      field('operator', choice('*', '/', '%')),
      field('right', $._expression),
    )),

    exponential_expression: $ => prec.right(PREC.EXPONENTIAL, seq(
      field('left', $._expression), '**', field('right', $._expression),
    )),

    unary_expression: $ => prec(PREC.UNARY, seq(
      field('operator', choice('!', 'not', '~', '-')),
      field('operand', $._expression),
    )),

    type_assertion_expression: $ => prec.left(PREC.TYPE_ASSERTION, seq(
      field('value', $._expression), 'as', field('type', $.type_annotation),
      optional($.comptime_field_overrides),
    )),

    // ========================================================
    // Postfix Expressions
    // ========================================================
    _postfix_expression: $ => choice(
      $.window_function_call,
      $.call_expression,
      $.member_expression,
      $.optional_member_expression,
      $.index_expression,
      $.try_expression,
      $._primary_expression,
    ),

    call_expression: $ => prec(PREC.POSTFIX, seq(
      field('function', $._postfix_expression),
      '(', optional($.arg_list), ')',
    )),

    member_expression: $ => prec(PREC.POSTFIX, seq(
      field('object', $._postfix_expression), '.',
      field('property', $.identifier),
    )),

    optional_member_expression: $ => prec(PREC.POSTFIX, seq(
      field('object', $._postfix_expression), '?.',
      field('property', $.identifier),
    )),

    index_expression: $ => prec(PREC.POSTFIX, seq(
      field('object', $._postfix_expression), '[', field('index', $._expression), ']',
    )),

    try_expression: $ => prec(PREC.POSTFIX, seq(
      field('operand', $._postfix_expression), token.immediate('?'),
    )),

    arg_list: $ => seq(commaSep1($.argument), optional(',')),

    argument: $ => choice($.named_arg, $._expression),

    named_arg: $ => seq(field('name', $.identifier), ':', field('value', $._expression)),

    // ========================================================
    // Primary Expressions
    // ========================================================
    _primary_expression: $ => choice(
      $.duration,
      $.datetime_expression,
      $._literal,
      $.array_literal,
      $.list_comprehension,
      $.object_literal,
      $.pattern_name,
      $.enum_constructor_expression,
      $.from_query_expression,
      $.comptime_for_expression,
      $.comptime_block,
      $.annotated_expression,
      $.async_let_expression,
      $.async_scope_expression,
      $.if_expression,
      $.if_block_expression,
      $.while_expression,
      $.for_expression,
      $.loop_expression,
      // $.let_expression,  // TODO: causes conflict with variable_declaration + pipe
      $.match_expression,
      $.block_expression,
      $.await_expression,
      $.function_expression,
      $.unit_literal,
      $.parenthesized_expression,
      $.typeof_expression,
      $.some_expression,
      $.self_expression,
      $.temporal_navigation,
      $.struct_literal,
      $.timeframe_expression,
      $.identifier,
    ),

    // ========================================================
    // Await / Async Expressions
    // ========================================================
    await_expression: $ => choice(
      seq('await', $.join_expression),
      seq('await', $._postfix_expression),
    ),

    join_expression: $ => seq(
      'join', field('kind', choice('all', 'race', 'any', 'settle')),
      '{', commaSep1($.join_branch), optional(','), '}',
    ),

    join_branch: $ => choice(
      seq(repeat1($.annotation), $.identifier, ':', $._expression),
      seq(repeat1($.annotation), $._expression),
      seq($.identifier, ':', $._expression),
      $._expression,
    ),

    // ========================================================
    // Struct / Enum Constructors
    // ========================================================
    struct_literal: $ => prec.dynamic(1, seq(
      field('type', $.identifier), '{', optional($.object_fields), '}',
    )),

    enum_constructor_expression: $ => seq(
      field('type', $.identifier), '::', field('variant', $.identifier),
      optional(choice(
        seq('(', optional($.arg_list), ')'),
        seq('{', optional($.object_fields), '}'),
      )),
    ),

    // ========================================================
    // Some / Typeof / Self
    // ========================================================
    some_expression: $ => seq('Some', '(', $._expression, ')'),
    typeof_expression: $ => seq('typeof', $._postfix_expression),
    self_expression: $ => 'self',

    // ========================================================
    // If / While / For / Loop / Let / Match Expressions
    // ========================================================
    if_expression: $ => prec.right(seq(
      'if', field('condition', $._expression),
      'then', field('consequence', $._expression),
      optional(seq('else', field('alternative', $._expression))),
    )),

    // if-block used as expression: let x = if cond { ... } else { ... }
    if_block_expression: $ => prec.right(prec.dynamic(10, seq(
      'if', field('condition', $._expression),
      '{', repeat($._statement), '}',
      'else', choice(
        $.if_block_expression,
        seq('{', repeat($._statement), '}'),
      ),
    ))),

    async_let_expression: $ => seq('async', 'let', field('name', $.identifier), '=', field('value', $._expression)),

    async_scope_expression: $ => seq('async', 'scope', $.block_expression),

    while_expression: $ => prec.dynamic(10, seq('while', field('condition', $._expression), $.block_expression)),

    for_expression: $ => seq(
      'for', optional('await'),
      field('pattern', $._match_pattern), 'in',
      field('iterable', $._expression),
      $.block_expression,
    ),

    loop_expression: $ => seq('loop', $.block_expression),

    let_expression: $ => seq(
      'let', field('pattern', $._match_pattern),
      optional(seq('=', field('value', $._expression))),
      'in', field('body', $._expression),
    ),

    match_expression: $ => prec.dynamic(10, seq(
      'match', field('value', $._expression),
      '{', repeat(seq($.match_arm, optional(','))), '}',
    )),

    match_arm: $ => seq(
      field('pattern', $._match_pattern),
      optional(seq('where', field('guard', $._expression))),
      '=>', field('value', $._expression),
    ),

    // ========================================================
    // Block Expression
    // ========================================================
    block_expression: $ => seq('{', repeat($._statement), '}'),

    // ========================================================
    // Match Patterns
    // ========================================================
    _match_pattern: $ => choice(
      $.pattern_literal,
      $.pattern_array,
      $.pattern_object,
      $.pattern_wildcard,
      $.pattern_constructor,
      $.pattern_identifier,
    ),

    pattern_literal: $ => $._literal,
    pattern_identifier: $ => $.identifier,
    pattern_wildcard: $ => '_',
    pattern_array: $ => seq('[', optional(commaSep1($._match_pattern)), ']'),
    pattern_object: $ => seq('{', optional(commaSep1($.pattern_field)), '}'),
    pattern_field: $ => choice(
      seq(field('key', $.identifier), ':', field('value', $._match_pattern)),
      field('key', $.identifier),
    ),

    pattern_constructor: $ => choice(
      // Qualified: Enum::Variant or Enum::Variant(...)
      seq(field('type', $.identifier), '::', field('variant', $.identifier),
          optional(choice(
            seq('(', optional(commaSep1($._match_pattern)), ')'),
            seq('{', optional(commaSep1($.pattern_field)), '}'),
          ))),
      // Unqualified: Some(x), Ok(x), Err(e)
      seq(field('name', choice('Some', 'Ok', 'Err', $.identifier)),
          choice(
            seq('(', optional(commaSep1($._match_pattern)), ')'),
            seq('{', optional(commaSep1($.pattern_field)), '}'),
          )),
    ),

    // ========================================================
    // Array / Object Literals
    // ========================================================
    array_literal: $ => seq('[', optional(seq(commaSep1(choice($.spread_element, $._expression)), optional(','))), ']'),

    list_comprehension: $ => seq(
      '[', field('body', $._expression),
      repeat1(seq('for', $._destructure_pattern, 'in', $._expression, optional(seq('if', $._expression)))),
      ']',
    ),

    spread_element: $ => seq('...', $._expression),

    object_literal: $ => seq('{', optional($.object_fields), '}'),

    object_fields: $ => seq(commaSep1($.object_field_item), optional(',')),

    object_field_item: $ => choice(
      $.object_spread,
      $.object_typed_field,
      $.object_value_field,
    ),

    object_value_field: $ => seq(field('key', $.identifier), ':', field('value', $._expression)),
    object_typed_field: $ => seq(field('key', $.identifier), ':', field('type', $.type_annotation), '=', field('value', $._expression)),
    object_spread: $ => seq('...', $._expression),

    // ========================================================
    // Data References
    // ========================================================
    // data references match via normal identifier + call/index expressions
    // e.g., data[0], data(1h)[5] are parsed as call_expression + index_expression

    // ========================================================
    // DateTime / Time References
    // ========================================================
    datetime_expression: $ => seq(
      '@', choice($.datetime_literal, choice('today', 'yesterday', 'now')),
      repeat(seq(choice('+', '-'), $.duration)),
    ),

    datetime_literal: $ => seq($.string, optional($.identifier)),

    time_reference: $ => seq('@', choice($.string, 'today', 'yesterday', 'now')),

    // ========================================================
    // Temporal Navigation
    // ========================================================
    temporal_navigation: $ => choice(
      seq('back', '(', $.number, optional($.time_unit), ')'),
      seq('forward', '(', $.number, optional($.time_unit), ')'),
    ),

    time_unit: $ => choice(
      'samples', 'sample', 'records', 'record',
      'minutes', 'hours', 'days', 'weeks', 'months',
      'minute', 'hour', 'day', 'week', 'month',
    ),

    // ========================================================
    // Timeframe Expression
    // ========================================================
    timeframe_expression: $ => seq('on', '(', $.timeframe, ')', '{', repeat($._statement), '}'),

    // ========================================================
    // LINQ-Style From Query Expression
    // ========================================================
    from_query_expression: $ => seq(
      'from', field('variable', $.identifier), 'in', field('source', $._postfix_expression),
      repeat(choice(
        seq('where', $._expression),
        seq('order', 'by', commaSep1(seq($._postfix_expression, optional(choice('asc', 'desc'))))),
        seq('group', $._postfix_expression, 'by', $._postfix_expression, optional(seq('into', $.identifier))),
        seq('join', $.identifier, 'in', $._postfix_expression, 'on', $._postfix_expression, 'equals', $._postfix_expression, optional(seq('into', $.identifier))),
        seq('let', $.identifier, '=', $._expression),
      )),
      'select', $._expression,
    ),

    // ========================================================
    // Pattern Name
    // ========================================================
    pattern_name: $ => seq('pattern', '::', $.identifier),

    // ========================================================
    // Misc Primary Expressions
    // ========================================================
    unit_literal: $ => seq('(', ')'),
    parenthesized_expression: $ => seq('(', $._expression, ')'),

    // ========================================================
    // Literals
    // ========================================================
    _literal: $ => choice(
      $.decimal_literal,
      $.percent_literal,
      $.number,
      $.string,
      $.boolean,
      $.none_literal,
      $.timeframe,
    ),

    percent_literal: $ => /[0-9]+(\.[0-9]+)?%/,
    decimal_literal: $ => token(seq(/[0-9]+(\.[0-9]+)?/, 'D')),

    boolean: $ => choice('true', 'false'),
    none_literal: $ => choice('None', 'null'),

    number: $ => /-?[0-9]+(\.[0-9]+)?/,
    integer: $ => /-?[0-9]+/,

    string: $ => choice(
      $.formatted_triple_string,
      $.formatted_string,
      $.triple_string,
      $.simple_string,
    ),

    formatted_triple_string: $ => token(seq('f', '"""', /([^"\\]|\\.|"[^"\\]|""[^"\\])*/, '"""')),
    formatted_string: $ => token(seq('f', '"', /([^"\\]|\\.)*/, '"')),
    triple_string: $ => token(seq('"""', /([^"\\]|\\.|"[^"\\]|""[^"\\])*/, '"""')),
    simple_string: $ => token(seq('"', /([^"\\]|\\.)*/, '"')),

    timeframe: $ => /[0-9]+(s|m|h|d|w|M)/,

    duration: $ => /[0-9]+(\.[0-9]+)?(s|m|h|d|w|M|y|samples|seconds|minutes|hours|days|weeks|months|years)/,

    // ========================================================
    // Identifier
    // ========================================================
    identifier: $ => token(/[a-zA-Z_][a-zA-Z0-9_]*/),
  },
});

// ========================================================
// Helpers
// ========================================================
function commaSep1(rule) {
  return seq(rule, repeat(seq(',', rule)));
}

function sep1(rule, separator) {
  return seq(rule, repeat(seq(separator, rule)));
}
