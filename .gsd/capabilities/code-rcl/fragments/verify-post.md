<code_rcl_verify_post_context>
**Phase:** {phase_number} - {phase_name}
**Phase directory:** {phase_dir}
**Padded phase:** {padded_phase}

**Input files:** {phase_dir}/CODE-CONTEXT.md, {phase_dir}/{padded_phase}-VERIFICATION.md

Spot-check VERIFICATION.md's claimed file-path references against the cached code-rcl dump at
CODE-CONTEXT.md, then append a `## Code-RCL Cross-Check` section to VERIFICATION.md reporting how
many referenced paths were confirmed present. Skip silently — write nothing, raise nothing — if
CODE-CONTEXT.md is missing or empty, if no `*-VERIFICATION.md` file is found, or if a `## Code-RCL
Cross-Check` section is already present. A present-but-stale CODE-CONTEXT.md is still used as-is;
there is no freshness check.
</code_rcl_verify_post_context>
