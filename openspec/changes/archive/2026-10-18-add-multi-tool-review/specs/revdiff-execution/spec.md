# Spec Delta

## REMOVED Requirements

### Requirement: Resolve the revdiff binary from the pane process environment

**Reason**: Binary detection is PATH-only for the whole three-tool catalog;
the `REVDIFF_BIN` override is removed by explicit decision (no escape hatch
for a herdr daemon PATH that lacks user tools — recorded as a rejected
risk).
**Migration**: reviewer-execution → "Tool binaries resolve from PATH only".

### Requirement: Validate and canonicalize the resolved path

**Reason**: The executable-bit check and canonicalization fold into the
generalized PATH-only resolution; the `REVDIFF_BIN`-specific hard error no
longer exists.
**Migration**: reviewer-execution → "Tool binaries resolve from PATH only".

### Requirement: Create the annotation output file under the state directory

**Reason**: Behavior is unchanged but now applies to all three tools, so it
moves to the tool-agnostic execution capability.
**Migration**: reviewer-execution → "Create the annotation output file under
the state directory".

### Requirement: Prune kept review outputs older than seven days at pane start

**Reason**: Behavior is unchanged but now applies to all three tools, so it
moves to the tool-agnostic execution capability.
**Migration**: reviewer-execution → "Prune kept review outputs older than
seven days at pane start".

### Requirement: Spawn revdiff as a direct child on the pane tty

**Reason**: A single revdiff-only spawn is replaced by the per-tool spawn
matrix (inherited tty for revdiff/tuicr, piped stdout for lumen).
**Migration**: reviewer-execution → "Each tool spawns as a direct child with
its own stdio strategy" and "revdiff keeps its file-based annotation
channel".

### Requirement: Read the child's exit code and annotation output

**Reason**: Result readback is now per-tool: revdiff keeps the file channel,
tuicr adds a post-exit session adapter, lumen adds a captured-stdout export.
**Migration**: reviewer-execution → "revdiff keeps its file-based annotation
channel", "tuicr annotations are collected after exit through the session
adapter", "lumen annotations are the captured stdout export, converted after
exit".
