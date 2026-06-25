#!/bin/sh
# SessionStart hook: inject the splinter "navigate the index before reading" rule
# into the model's context. Non-blocking — it only adds a standing reminder.
cat <<'JSON'
{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"When the splinter MCP tools are connected, navigate the function index before reading source files: use search_bodies(query) or open_source(path) to locate code, read_splinter(path) for prior notes, and read_body(path) to load a single function. Use Read/Grep on the whole source file only if those are not enough. Record durable findings with write_splinter(path, note)."}}
JSON
