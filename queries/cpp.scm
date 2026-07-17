(function_definition
  declarator: (function_declarator declarator: (identifier) @name)
  body: (compound_statement) @body) @def
(function_definition
  declarator: (function_declarator declarator: (field_identifier) @name)
  body: (compound_statement) @body) @def
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier
      scope: (namespace_identifier) @qualifier
      name: (identifier) @name))
  body: (compound_statement) @body) @def
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator declarator: (identifier) @name))
  body: (compound_statement) @body) @def

(class_specifier name: (type_identifier) @container.name) @container
(struct_specifier name: (type_identifier) @container.name) @container
