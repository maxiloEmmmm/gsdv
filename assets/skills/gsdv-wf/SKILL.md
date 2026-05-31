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

**Root project: `$GSDV_SPEC`/root.md**

The root project mainly indexes the whole project overview. If it does not exist, stop. It is called ROOT_MD below.

**Subproject: `$GSDV_SPEC`/ps/*/root.md**

Example: `$GSDV_SPEC`/ps/some/root.md
`some/root.md` describes the core mechanism of this project. It is called P_MD below.

<some/task-*.md>
The task description Markdown files for the some project. They are called P_TASK_MD below.

**This Markdown layout is**

desc... about this task before first step.

## [x] step1 title
Step description...

## [ ] step2 title
Step description...

- Each step is a Markdown level-2 heading with a checkbox: `## [ ] title` or `## [x] title`.
- A step title is exactly one heading line. It may contain spaces or Chinese text, but must not contain a newline.
- The step description is the Markdown content after the step heading and before the next step heading.
- A step description may contain implementation notes, reasoning, pseudocode, constraints, or acceptance details.
</some/task-*.md>
</desc>


<route>
<r.next>
  1. find the first unchecked step across all project tasks, ordered by project name and task name.
  2. check the current implementation progress of this step, then continue implementing it based on the step title and step description.
  3. mark the step as complete only when the implementation is actually finished and no known blocker remains. If the work is partial, blocked, or needs user confirmation, do not mark it complete.
  4. if entry args include loop:true
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
    after user confirms, fill the steps into the task file as Markdown level-2 checkbox headings
</r.step>

<r.other>
  if the entry args do not match any route above, follow the user's intent from the entry args
  if user gives a full workflow logical path like: project > task > step
    run step 2 of `r.next` directly for that target step
  if entry args not clear
    ask what the user wants to do
</r.other>
</route>
