(function_declaration name: (identifier) @name body: (block) @body) @def

(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @qualifier
        (pointer_type (type_identifier) @qualifier)
      ]))
  name: (field_identifier) @name
  body: (block) @body) @def
