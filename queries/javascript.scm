(function_declaration name: (identifier) @name body: (statement_block) @body) @def
(generator_function_declaration name: (identifier) @name body: (statement_block) @body) @def
(method_definition name: (property_identifier) @name body: (statement_block) @body) @def

(lexical_declaration
  (variable_declarator
    name: (identifier) @name
    value: [
      (arrow_function body: (statement_block) @body)
      (function_expression body: (statement_block) @body)
    ])) @def
(variable_declaration
  (variable_declarator
    name: (identifier) @name
    value: [
      (arrow_function body: (statement_block) @body)
      (function_expression body: (statement_block) @body)
    ])) @def

(class_declaration name: (identifier) @container.name) @container
