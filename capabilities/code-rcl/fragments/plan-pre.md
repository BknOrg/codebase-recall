<code_context_context>
**Phase:** {phase_number} - {phase_name}
**Phase directory:** {phase_dir}
**Padded phase:** {padded_phase}

**Output file:** {phase_dir}/CODE-CONTEXT.md

Run code-rcl's default full-project dump and write it atomically (temp file + rename) to the
output file above. Skip silently — write nothing, raise nothing — if `code-rcl` is not on PATH or
the dump fails. Never scope the dump to phase-relevant files only; this is always a full-project
dump so the planner sees complete architectural context.
</code_context_context>
