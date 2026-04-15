# Eris — quick start for users

Eris is a **local**, vault-centric assistant: you keep your notes in a folder (the **vault**), run `eris chat` from that folder, and chat in the terminal or in a small browser window on your machine. Ollama powers the model; optional Qdrant holds semantic memory over your notes.

This page is for **people installing a pre-built binary**. Developers building from source should use the [project README](../README.md).

---

## 1. Install the binary so the shell can find `eris`

After you unpack the download, you have a single executable named **`eris`**. Your shell only knows how to run it by name if the file sits in a directory on your **`PATH`**.

### Recommended: `/usr/local/bin` (macOS and Linux)

`/usr/local/bin` is a conventional place for third-party programs and is usually already on `PATH`.

1. Move or copy the binary there (you will need administrator rights for this location):

   ```bash
   sudo cp /path/to/your/download/eris /usr/local/bin/eris
   sudo chmod +x /usr/local/bin/eris
   ```

2. Confirm:

   ```bash
   which eris
   eris --help
   ```

**macOS note:** Binaries downloaded from the internet may be quarantined by Gatekeeper. If the system refuses to run `eris`, you can clear the quarantine attribute (still your responsibility to trust the file):

```bash
sudo xattr -c /usr/local/bin/eris
```

You may still see a one-time privacy prompt depending on your security settings.

### Alternative: your home directory (no `sudo`)

If you prefer not to use `/usr/local/bin`, use a folder under your home directory and add it to `PATH`:

```bash
mkdir -p "$HOME/bin"
mv /path/to/your/download/eris "$HOME/bin/eris"
chmod +x "$HOME/bin/eris"
```

Then append **one** of the following to `~/.zshrc` (macOS default) or `~/.bashrc` (many Linux setups), open a new terminal, and run `which eris` again:

```bash
export PATH="$HOME/bin:$PATH"
```

Eris itself may suggest a similar layout if it detects the binary still living under **Downloads** or **/tmp** on first launch.

---

## 2. What you need running before the first real chat

Eris expects **Ollama** for chat and embeddings. For full semantic memory (and the default strict startup), it also expects **Qdrant** reachable at the address in your config (defaults are described in the [main README](../README.md#prerequisites)).

Minimal checklist:

1. Install [Ollama](https://ollama.com) and start it (the installer usually leaves a background service running).
2. Pull at least one **chat** model and one **embedding** model, matching what you will select or configure (defaults are documented in the main README).
3. If you use Qdrant: run it (for example via Docker) or let Eris try to start it when possible; otherwise adjust `require_semantic_brain` in `.fcp/config.toml` once that file exists.

Details and environment variables live in the [Prerequisites](../README.md#prerequisites) section of the main README.

---

## 3. First launch: your “welcome” flow

Eris treats the directory you launch from as the **vault root** (where `.fcp/` and your notes live). There is no separate global “install folder” for your data.

### Step A — Create a vault and enter it

```bash
mkdir -p ~/eris/vaults/MyVault
cd ~/eris/vaults/MyVault
```

### Step B — Start chat

```bash
eris chat
```

If this vault has **never** been initialized (no seal yet), you will see a short **first-run sequence** in the terminal before the full-screen UI appears. Think of it as a welcome and setup wizard, not a separate graphical splash screen.

**Setup welder (environment and vault check)** — when standard input is an interactive terminal and you have not opted out, Eris may:

- Ask you to confirm that the **current directory** is the vault you want.
- Let you set a **workspace id** (used for Qdrant partitioning and related labels).
- Probe **Ollama** and **Qdrant** and print hints if something is missing (for example links to install Docker or Ollama).

If you decline the suggested vault directory, create a folder, `cd` into it, and run `eris chat` again from there.

**Ignition (identity and config)** — next, you will be prompted for:

- **Agent name** (how the assistant is labeled in your vault identity file).
- **Your name** (optional).
- **Ollama model** (from the models Ollama reports as installed, or a manual default if the list is empty).

Eris then creates the vault layout (including `00_Invariants/` and related folders), writes **`.fcp/config.toml`**, and places a **seal** file so the next launch skips this wizard.

**Main interface** — after ignition, the **terminal UI** takes over the screen. You may briefly see startup lines about **peripheral readiness** (Ollama and Qdrant). Then you use the chat area as documented in the main README.

Same vault in a **browser** (still local):

```bash
cd ~/eris/vaults/MyVault
eris chat --web
```

---

## 4. Everyday use

Always **`cd` into your vault** before starting Eris so paths and `.fcp/config.toml` resolve correctly:

```bash
cd ~/eris/vaults/MyVault
eris chat
```

Logs and telemetry go under **`.fcp/telemetry/`** inside that vault, not to the terminal as ordinary `print` output.

For flags (`--web`, workspace overrides, verbosity), run:

```bash
eris chat --help
```

---

## 5. Where to read more

- Full prerequisites, architecture, tool behavior: [README.md](../README.md)
- Optional: skip the interactive welder in automation with environment variables (see code and tracing for `ERIS_SKIP_SETUP` and `CI` behavior in the developer docs if you need non-interactive runs).

If something fails on first launch, read the message in the terminal carefully: it often names the missing piece (Ollama API, Qdrant, or vault directory).
