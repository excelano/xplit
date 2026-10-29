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

## Scrollback

Each pane keeps the last 10,000 lines that scrolled off its top. While a pane is scrolled back, its cursor is hidden, a dim `+N` in its top-right corner says how many lines back you are, and the view holds still as new output arrives. Any key other than Alt+PageUp and Alt+PageDown returns the pane to the bottom and goes to the shell as usual. Full-screen programs such as vim, less and htop have no scrollback of their own, so while one is running the two keys go to it instead.

## Behavior to know about

xplit does not forward mouse events, so mouse support in programs like htop does not work inside a pane. The mouse wheel reaches the focused pane as arrow keys, the way GNOME Terminal treats any full-screen program, so at a shell prompt it steps through command history rather than scrollback. Programs that query the terminal (for its cursor position or its identity) get no answer and fall back to their defaults.

## Ideas for later

- Mouse support: wheel scrolling through scrollback, and clicks forwarded to the pane they land in, with Shift+drag left for selecting text.
- Passing cursor shape, the bell, and window title changes through from the focused pane to the real terminal.
- Answering terminal queries such as cursor position reports from each pane's emulated screen.
- Non-blocking writes to the panes, so a program that stops reading input cannot stall keystrokes bound for the other pane.
- A keyboard way to move the divider off the halfway mark.
