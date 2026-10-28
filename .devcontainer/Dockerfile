FROM mcr.microsoft.com/devcontainers/base:ubuntu

# Create the directory and set permissions
RUN mkdir -p /home/vscode/.vscode-server/extensions \
             /home/vscode/.vscode-server/bin \
    && chown -R vscode:vscode /home/vscode/.vscode-server

# Install apt
RUN apt update

# Install system package
RUN apt-get install -y build-essential

# Phase 0 detector spike: ffmpeg drives every decode, numpy/Pillow do the frame
# analysis. ffmpeg is a hard dependency of the product itself (see CLAUDE.md 3.1).
RUN apt-get install -y ffmpeg python3-numpy python3-pil

# Native egui window and CPAL audio; FFmpeg is a separate process. No browser
# engine, Node, GTK or GStreamer development stack is required.
RUN apt-get install -y libasound2-dev libxkbcommon-dev libxkbcommon-x11-0 libx11-dev libgl1-mesa-dev \
    file pkg-config xvfb xdotool imagemagick xz-utils

USER vscode

# Rust toolchain for the Linux port (fpv-rs/, CLAUDE.md 9.9). rustup picks the
# host architecture itself, so the arm64 trap above does not apply here.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --component clippy,rustfmt
ENV PATH="/home/vscode/.cargo/bin:${PATH}"

# Install Claude Code
RUN curl -fsSL https://claude.ai/install.sh | bash

# Install OpenCode
RUN curl -fsSL https://opencode.ai/install | bash
