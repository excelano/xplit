# xplit

xplit splits one terminal window into two shells and does nothing else. There is no prefix key, no status bar, no configuration and no sessions to manage, so the only thing between you and the shells is a dim one-character divider. It is for the moment you want a second shell next to the first without reaching for tmux and its keybindings.

It runs on Linux and is built against GNOME Terminal.

## Install

```sh
cargo install --path .
```

## Usage

```sh
xplit        # two panes side by side
xplit down   # two panes stacked top and bottom
```

Each pane runs `$SHELL` (or `/bin/bash` when it is unset) in the directory xplit was started from, with `TERM=xterm-256color`. When a pane's shell ends, by `exit`, Ctrl+D or anything else, the pane closes and the other one takes the full window. When the last pane closes, xplit puts the terminal back as it found it and exits. Resizing the window resizes both panes.

## Keys

| Layout       | Keys                     | Action                       |
|--------------|--------------------------|------------------------------|
| side by side | Alt+Left / Alt+Right     | focus the left or right pane |
| stacked      | Alt+Up / Alt+Down        | focus the top or bottom pane |
| either       | Alt+PageUp / Alt+PageDown | scroll the focused pane back or forward a page |

Every other key goes to the focused pane unchanged, including Ctrl+C, Ctrl+Z, Esc and bracketed paste. The Alt+arrow pair that does not belong to the current layout is passed through to the shell too, and so are all Alt+arrows once only one pane is left. Only the focused pane shows a cursor.

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
