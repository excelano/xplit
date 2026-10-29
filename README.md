# xplit

xplit splits one terminal window into two shells and does nothing else. There is no prefix key, no status bar, no configuration and no sessions to manage, so the only thing between you and the shells is a dim one-character divider. It is for the moment you want a second shell next to the first without reaching for tmux and its keybindings.

It runs on Linux and is built against GNOME Terminal.

## Install

```sh
cargo install --path .
```

## Usage

```sh
xplit                  # two panes side by side
xplit down             # two panes stacked top and bottom
xplit solo             # one pane, ready to split
xplit claude           # a shell on the left, claude on the right
xplit down htop        # a shell on top, htop below
xplit hide             # inside xplit: fold away the pane you typed it in
xplit -- down          # run a program that happens to be called down
```

Each pane runs `$SHELL` (or `/bin/bash` when it is unset) with `TERM=xterm-256color`, unless it was opened with a command, in which case it runs that command. When whatever a pane runs ends, by `exit`, Ctrl+D or anything else, the pane closes and the pane it was split from takes back its space. When the last pane closes, xplit puts the terminal back as it found it and exits. Resizing the window resizes every pane.

### Inside xplit

Typed in one of its own panes, `xplit` does not start a second copy. It asks the xplit it is running in to split the pane you typed in: that pane stays on the left (or on top, with `down`), and the new pane opens beside it with the focus, in the directory and with the environment of the shell you typed in. A command after the verb runs in the new pane, so `xplit claude` from inside a pane opens Claude Code next to it. Any pane can be split again, in either direction, so a screen can hold as many panes as fit; xplit refuses a split that would leave a half too small to use. `xplit solo` refuses because you are already in xplit.

`xplit hide` folds the pane you typed it in down to a dim dashed line (`┆` or `┄`) along its edge, and the pane beside it takes the space. What runs in a hidden pane keeps running at the size it had, so it never sees a resize. Alt+arrow toward the dashed line, or a click on it, brings the pane back with the focus. When both halves of a split are hidden they fold into one line and come back together. The last visible pane cannot be hidden.

To have a window that starts in xplit without changing every terminal, give GNOME Terminal a second profile (Preferences, then the + beside Profiles) whose Command tab runs `xplit solo` as a custom command instead of the shell, and open it from the new-tab menu or with `gnome-terminal --profile=<name>`. It looks like an ordinary shell until you type `xplit`.

## Keys

| Keys                      | Action                                          |
|---------------------------|-------------------------------------------------|
| Alt+arrow                 | focus the next pane in that direction, or bring back a hidden one |
| Alt+PageUp / Alt+PageDown | scroll the focused pane back or forward a page  |

Alt+arrow moves to the nearest pane on that side, and where several panes border it, to the one level with the cursor. When there is no pane in that direction, the key goes to the program in the focused pane, so Alt+arrows still reach the shell whenever they have nowhere to move. Every other key goes to the focused pane unchanged, including Ctrl+C, Ctrl+Z, Esc and bracketed paste. Only the focused pane shows a cursor.

## Mouse and copying

xplit handles the mouse itself, so a selection stays inside the pane it started in. Drag to select; the selection stops at the pane's edges, and soft-wrapped lines come back joined. Copy it to the clipboard with a right-click, or with Ctrl+C while the selection is showing; with nothing selected, Ctrl+C goes to the shell as an interrupt as usual. Finishing a drag also sets the primary selection, and a middle-click pastes the primary selection into the pane under the mouse. Typing, clicking, scrolling or resizing clears the selection.

Clicking a pane focuses it. The wheel scrolls the pane under the mouse through its scrollback, and in a full-screen program without mouse support it sends arrow keys, so less and man scroll with it. Programs that ask for the mouse, such as htop and vim with `mouse=a`, receive clicks, drags and the wheel at the right place in their pane. To select text in one of those, hold Ctrl while dragging. Shift+drag still makes GNOME Terminal's own selection, which runs across both panes.

Copying and pasting go through `wl-copy` and `wl-paste` under Wayland and `xclip` under X11, from the `wl-clipboard` and `xclip` packages. Without them, or in a session with neither display (such as over ssh), selecting still works and copying does nothing.

## Scrollback

Each pane keeps the last 10,000 lines that scrolled off its top. While a pane is scrolled back, its cursor is hidden, a dim `+N` in its top-right corner says how many lines back you are, and the view holds still as new output arrives. Any key other than Alt+PageUp and Alt+PageDown returns the pane to the bottom and goes to the shell as usual. Full-screen programs such as vim, less and htop have no scrollback of their own, so while one is running the two keys go to it instead.

## Behavior to know about

xplit reports mouse movement to programs only while a button is held, so a program that tracks the pointer as it hovers sees no hover events. Programs that query the terminal (for its cursor position or its identity) get no answer and fall back to their defaults.

## Ideas for later

- Scrolling the pane while a drag selection runs past its top or bottom edge.
- Keeping a selection attached to its text when new output scrolls the pane, rather than to its position on screen.
- Passing cursor shape, the bell, and window title changes through from the focused pane to the real terminal.
- Answering terminal queries such as cursor position reports from each pane's emulated screen.
- Non-blocking writes to the panes, so a program that stops reading input cannot stall keystrokes bound for the other pane.
- A keyboard way to move the divider off the halfway mark.
