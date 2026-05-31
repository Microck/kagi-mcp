# Packet P1: Discovery

## Objective
Inspect the repo and local `kagi` CLI capabilities to find a safe, transparent implementation path.

## Context
The user asked for a stealth-like option and referenced CloakBrowser. The accepted scope is privacy-preserving request controls only, not anti-detection behavior.

## Do
- Inspect `src/main.rs`, `Cargo.toml`, README test instructions, and local `kagi` help.
- Identify existing CLI flags that can support privacy-oriented behavior.
- Propose code and tests.

## Do Not
- Edit files in this packet.
- Propose fingerprint spoofing, CAPTCHA avoidance, proxy rotation, or bot-detection evasion.

## Expected Output
A concise result with file anchors, safe feature shape, files to edit, and validators.
