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

--steps--
- [x]..
- xxx1
  - [ ] a
    xx desc..
  - [ ] b
    xx desc..
- xxx2
  xx desc...
  
---------

--doc--
- xxx1
  xx desc..
- xxx2
  xx desc..
--------

- Each doc item is an independent logical unit inside the task.
- A step may be associated with doc, or it may be independent.
- A step may or may not have child steps. Each step and doc item can have its own desc, usually mixing logical reasoning and pseudocode.
- Each top-level step item may have a corresponding key in doc. If no matching doc key exists, it is an independent step. Child steps have no corresponding doc and inherit their parent by default.
- doc and step keys should use English, such as a.b.c. Do not use spaces or newlines. Keep keys short and unique; avoid meaningless repeated key prefixes within the same task.
- A step can have at most one level of child steps; child steps cannot have their own child steps.
</some/task-*.md>
</desc>


<route>
<r.next>
  1. find first unchecked leaf step on all project's task, order by project name and task name.
  2. check the current implementation progress of this step, see whether there is a step-level description, continue implementing this step based on the step documentation and its own description.
  3. mark the step as complete only when the implementation is actually finished and no known blocker remains. If the work is partial, blocked, or needs user confirmation, do not mark it complete.
  4. if entry args include loop:true
     repeat r.next after each completed step
     stop when no unchecked leaf step remains, work is blocked, or user confirmation is needed
</r.next>

<r.step>
  find project task doc and target step by entry args
  if task doc can't be identified
    ask user which task doc to use, then stop
  if no step exists in task doc
    need_fill_step = true
  else
    ask user whether current steps are enough
    need_fill_step = user's answer is no
  if need_fill_step
    propose a `plan` with detected steps from task doc and wait for user confirmation
    after user confirms, fill the steps into task doc
</r.step>

<r.other>
  entry args not match before route, just flow user by entry args
  if user give full workflow logical path like: project > task ...
    call `r.next` and direct run it's step 2
  if entry args not clear
    ask user want to do?
</r.other>
</route>
