Set Shell bash
Set Width 1400
Set Height 900
Set FontFamily "CaskaydiaCove Nerd Font Mono"
Set FontSize 22
Set Padding 20
Set TypingSpeed 0
Env NO_COLOR ""

Type "./target/release/hntui"
Enter
Wait+Screen /Top Stories/
Sleep 2s
Enter
Sleep 5s
# VHS captures the next frame; wait before changing the screen.
Screenshot screenshots/comments.png
Sleep 500ms
Type "q"
Sleep 500ms
Type "q"
