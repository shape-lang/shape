; Inject foreign function bodies based on their declared language.
; Example: `fn python foo() { ... }` injects `python` into the body node.
((foreign_function_definition
   language: (foreign_language_identifier) @injection.language
   body: (foreign_body) @injection.content)
 (#set! injection.include-children))
