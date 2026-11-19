# fish for Shabnam — install as /root/.config/fish/config.fish
set -g fish_greeting
if status is-interactive
    set -gx EDITOR hx
    alias ll 'ls -lah'
end
