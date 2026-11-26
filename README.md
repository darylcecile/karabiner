# Karabiner

Karabiner helps you turn a software task into a plan, changes to your project and a review of the results. It works with a coding agent you already use, while giving you a Mac app to organize the work, follow progress and answer questions.

## Get the preview

Karabiner is currently a preview. **There is no public packaged download or Mac installer yet.** Ask the maintainer for a prebuilt preview copy.

You will need:

- An **Apple silicon Mac**—one with an M-series chip.
- A supported coding agent installed and signed in, such as **GitHub Copilot CLI, Claude Code or Codex CLI**. These are coding tools that can work on your project files; the ordinary ChatGPT or Claude chat app alone is not enough. Use the coding agent's own setup instructions, or ask the maintainer for help preparing it.

A supplied preview is already built. You do not need developer tools to use it.

## Open Karabiner

The preview has two files: **`karabiner`**, the desktop app, and **`karabiner-service`**, its companion. The companion saves your work and manages running tasks. You start it yourself each time you want to use Karabiner.

1. Save both supplied files somewhere you can find again. If they arrive in a compressed archive, open the archive in Finder first.
2. In Finder, double-click **`karabiner-service`**. It opens a Terminal window with status text. Wait for the word **ready**, then leave that window open. If the companion is already running, use that instance.
3. Double-click the supplied **`karabiner`** desktop executable. The app window should appear. This preview uses executable files rather than a usual Mac app installer, so it may also open a Terminal window; leave that open too.

You do not need to type commands into those windows. If Finder cannot open the supplied files, or the app window does not appear, ask the maintainer for help with your preview copy.

## Set up your first project

A **project folder** contains the files you want your coding agent to work on. A **workflow** is the sequence of steps it follows, such as Plan → Implement → Review.

1. In Karabiner, open **Projects → New project**, or choose **Templates** in the sidebar. Select **Software delivery**, then **Use template**.
2. Give the project a name and choose **Standalone** to start with a project on your Mac.
3. Set **Local checkout** to your existing project folder. This is a text field: in Finder, select the folder and press **Option–Command–C** to copy its full location, then paste it into the field.
4. Under **Agent harness**, choose the coding agent you installed, then click **Create project**. The Software delivery template uses Copilot when you leave this as **As template**.
5. Open the project's **Settings → Agents**. Select each agent and click **Check readiness**. Follow any message about installation or signing in, then check again. Click **Save** after changing settings. Start with **Settings → Environment → Local host**.
6. Open **Workflow** to see the Plan, Implement and Review steps. You can adjust their instructions. After making changes, click **Save**, then **Validate**, and look for **Valid**.

## Give it a task

1. Open the project's **Board** and choose **Add task** or a column's **+** button.
2. Enter what you want done—for example, “Add a contact form to the website”—and press Enter. Open the task to add more detail about the result you want.
3. Check the task's **Workflow**, then open **Runs** and click **Start run**.
4. Follow the steps in **Runs**. Answer questions and permission requests there. Use **Add instruction** if you need to guide the work while it is running.
5. When it finishes, open the run's results and change reviews. Review the changes in your project folder before deciding what to do next.

Creating a project or task does not start an agent by itself. **Start run** begins the work unless you have deliberately enabled automatic scheduling.

## Pause, stop and come back later

- Use **Pause** in **Runs** to pause at a stopping point between steps, then **Resume** to continue. Keep the companion open while work is paused.
- Use **Cancel** to stop a run. Closing the app window alone does not stop work managed by the companion.
- When you have finished, stop any active work, close the app window and respond to any unsaved-changes prompt. Click the companion's Terminal window and press **Control–C**. If the app also opened a Terminal window, press **Control–C** there before closing it too.
- Next time, open the companion first and then Karabiner. Your saved projects, tasks and history will still be there. If a run was interrupted by a shutdown, review its existing changes; the app may offer **Retry** instead of Resume.

Karabiner stores your project settings, tasks and history on your Mac. Your project files stay in the folder you chose. Your coding agent continues to use its own account and online provider when carrying out work.

## If something is not working

- **The app cannot connect:** check that the companion's Terminal window is still open and says **ready**. Open the companion if it has stopped, then click **Retry** in the app. If the companion shows an error, share that message with the maintainer.
- **An agent is not ready:** open **Settings → Agents → Check readiness**. Make sure the selected coding tool is installed and signed in with an account that can use it.
- **A task will not start:** check **Local checkout** under **Settings → General**, confirm that the workflow shows **Valid**, and check the agent's readiness. The message in **Runs** can help explain what needs attention.

---

[Developer instructions](.groundwork/guides/project.md#development)
