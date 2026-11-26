# Karabiner

Karabiner helps you turn a software task into a plan, changes to your project and a review of the results. It works with a coding agent you already use, while giving you a Mac app to organize the work, follow progress and answer questions.

## Get Karabiner

[**Download Karabiner for Apple silicon**](https://github.com/darylcecile/karabiner/releases/latest/download/Karabiner-macos-arm64.zip). The client is the only download you need; it sets up its matching background component automatically. See [Releases](https://github.com/darylcecile/karabiner/releases) for release notes.

You will need:

- An **Apple silicon Mac**—one with an M-series chip.
- A supported coding agent installed and signed in, such as **GitHub Copilot CLI, Claude Code or Codex CLI**. These are coding tools that can work on your project files; the ordinary ChatGPT or Claude chat app alone is not enough. Use the coding agent's own setup instructions, or ask the maintainer for help preparing it.

The downloaded app is already built. You do not need developer tools to use it.

## Open Karabiner

1. Open the downloaded ZIP in Finder.
2. Move **Karabiner.app** to your **Applications** folder, then open it.
3. On first launch, Karabiner shows **Setting up** while it downloads and starts the matching background component. Leave the app open until setup finishes. On later launches, it reuses that component when available.

The background component saves your work and manages tasks. Karabiner handles it for you; there is no second program to download or start manually. First-time setup needs an internet connection.

This preview is not Developer ID-signed or notarized. macOS may ask you to confirm opening it or may block it. Follow the message shown by macOS, or ask the maintainer for help with the preview.

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

- Use **Pause** in **Runs** to pause at a stopping point between steps, then **Resume** to continue.
- Use **Cancel** to stop a run. Closing the app window alone does not cancel work; the background component continues managing it.
- When you have finished, stop any work you do not want to continue, close the app window and respond to any unsaved-changes prompt.
- Next time, open Karabiner again. Your saved projects, tasks and history will still be there, and Karabiner starts its background component if needed. If a run was interrupted by a computer shutdown, review its existing changes; the app may offer **Retry** instead of Resume.

Karabiner stores your project settings, tasks and history on your Mac. Your project files stay in the folder you chose. Your coding agent continues to use its own account and online provider when carrying out work.

## If something is not working

- **Setup or connection failed:** check your internet connection, read the message in Karabiner and click **Retry**. If it still fails, share that message and the release you downloaded with the maintainer. Karabiner leaves an existing incompatible background component alone rather than replacing it.
- **An agent is not ready:** open **Settings → Agents → Check readiness**. Make sure the selected coding tool is installed and signed in with an account that can use it.
- **A task will not start:** check **Local checkout** under **Settings → General**, confirm that the workflow shows **Valid**, and check the agent's readiness. The message in **Runs** can help explain what needs attention.

---

[Contributor guide](CONTRIBUTING.md)
