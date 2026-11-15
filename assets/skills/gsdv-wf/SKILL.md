---
name: gsdv-wf
description: "Lightweight task management"
---

<entry-args>
- The first argument is `$command`; route to the matching handler by `$command`. If no route matches, jump to `r.other`.
- Later arguments are consumed by the matched route.
</entry-args>

<desc>
This is a lightweight task management convention. The core directory `$GSDV_SPEC` is fixed as ./gsdv-spec.

**Root project: `$GSDV_SPEC`/`<repo_id>.root.md`**

`repo_id` is exactly 4 lowercase ASCII letters or digits. The root project mainly indexes the whole project overview. If exactly one valid root file does not exist, stop. It is called ROOT_MD below.

**Subproject: `$GSDV_SPEC`/ps/`<project_id>.<project_name>`/root.md**

`project_id` is exactly 4 lowercase ASCII letters or digits and is unique within the repo. Example: `$GSDV_SPEC`/ps/a1b2.some/root.md`. The file describes the core mechanism of this project. It is called P_MD below.

**Task: `<project_id>.<project_name>`/`<task_id>.task-<task_name>.md`**

`task_id` is exactly 4 lowercase ASCII letters or digits and is unique within its project. These task description Markdown files are called P_TASK_MD below.

**This Markdown layout is**

desc... about this task before first step.

## [x]a1b2c3 step1 title
Step description...

## [ ]d4e5f6 step2 title
Step description...

- Each step is a Markdown level-2 heading with a checkbox immediately followed by a stable ID: `## [ ]<step_id> title` or `## [x]<step_id> title`.
- `step_id` is exactly 6 lowercase ASCII letters or digits and is unique within its task, including nested steps.
- A step title is exactly one heading line. It may contain spaces or Chinese text, but must not contain a newline.
- The step description is the Markdown content after the step heading and before the next step heading.
- A step description may contain implementation notes, reasoning, pseudocode, constraints, or acceptance details.
- Preserve every existing repo, project, task, and step ID when editing or renaming. Never derive identity from a display name.
- When creating an item, generate a random lowercase `0-9a-z` ID of the required length and retry until it is unique in the stated scope.
- Treat malformed checkbox headings, invalid IDs, duplicate IDs, multiple repo roots, or mixed legacy/stable roots as invalid workflow data; do not silently parse around them.
- Legacy `root.md`, `ps/<name>`, `task-*.md`, and `## [ ] title` data is migrated lazily by GSDV when opened. Do not create new legacy-format data.

<some/docs>
some docs or prd can @ref in `*.task-*.md`
</some/docs>

</desc>


<route>
<r.next>
  1. find the first unchecked step across all project tasks, ordered by project name and task name.
  2. if step desc is description, first `plan` with user, and fill to step description.
  3. check the current implementation progress of this step, then continue implementing it based on the step title and step description.
  4. mark the step as complete only when the implementation is actually finished and no known blocker remains. If the work is partial, blocked, or needs user confirmation, do not mark it complete.
  5. if entry args include loop:true
     repeat r.next after each completed step
     stop when no unchecked step remains, work is blocked, or user confirmation is needed
</r.next>

<r.step>
  find the project task file and target step from entry args
  if the task file can't be identified
    ask user which task file to use, then stop
  if no step exists in the task file
    need_fill_step = true
  else
    ask user whether current steps are enough
    need_fill_step = user's answer is no
  if need_fill_step
    propose a `plan` with detected steps from the task file and wait for user confirmation
    after user confirms, fill the steps into the task file as Markdown level-2 checkbox headings with unique 6-character step IDs
</r.step>

<r.other>
  if the entry args do not match any route above, follow the user's intent from the entry args
  if user gives a full workflow logical path like: project > task > step
    run step 2 of `r.next` directly for that target step
  if entry args not clear
    ask what the user wants to do
</r.other>
</route>
