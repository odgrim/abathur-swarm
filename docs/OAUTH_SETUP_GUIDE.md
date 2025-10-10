# OAuth Authentication Setup Guide

## Overview

Abathur now supports **dual-mode authentication**: API keys and OAuth tokens. This guide will help you set up and use OAuth authentication with Claude Max subscriptions.

---

## Quick Start

### Option 1: API Key (Recommended for Most Users)

```bash
# Set your API key
abathur config set-key sk-ant-api03-your-key-here

# Verify configuration
abathur config oauth-status
```

**Advantages:**
- ✅ Unlimited context window (1M tokens)
- ✅ No rate limits (pay-per-token pricing)
- ✅ Simple setup

**Use cases:** Production workloads, large context tasks, high-volume usage

---

### Option 2: OAuth (For Claude Max/Pro Subscribers)

```bash
# Login with OAuth
abathur config oauth-login --manual

# Follow the prompts:
# Access token: [paste your token]
# Refresh token: [paste your refresh token]
# Expires in: 3600

# Verify authentication
abathur config oauth-status
```

**Advantages:**
- ✅ Use your existing Claude subscription
- ✅ Automatic token refresh
- ✅ No additional API costs

**Limitations:**
- ⚠️ Context window: 200K tokens (vs 1M for API key)
- ⚠️ Rate limits apply (50-200 prompts per 5-hour window)

**Use cases:** Development, testing, personal projects, Claude Max subscribers

---

## Obtaining OAuth Tokens

### From Claude Code

1. Open Claude Code
2. Access developer settings
3. Copy your OAuth access token and refresh token

### From console.anthropic.com

1. Log in to [console.anthropic.com](https://console.anthropic.com)
2. Navigate to API settings
3. Generate OAuth credentials
4. Copy the access token and refresh token

---

## Authentication Priority

When both authentication methods are configured, Abathur uses this priority:

1. **API Key** (highest priority)
   - Environment variable: `ANTHROPIC_API_KEY`
   - System keychain: `anthropic_api_key`
   - `.env` file: `ANTHROPIC_API_KEY=...`

2. **OAuth Tokens** (fallback)
   - Environment variables: `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_REFRESH_TOKEN`
   - System keychain: `anthropic_oauth_access_token`, `anthropic_oauth_refresh_token`
   - `.env` file: `ANTHROPIC_AUTH_TOKEN=...`

3. **Error** (no authentication configured)

---

## CLI Commands

### `oauth-login` - Authenticate with OAuth

```bash
# Manual token input (recommended)
abathur config oauth-login --manual

# Store in .env file instead of keychain
abathur config oauth-login --manual --no-use-keychain
```

**Options:**
- `--manual`: Enable manual token input mode (required)
- `--use-keychain / --no-use-keychain`: Store in system keychain (default: True)

**Example:**
```bash
$ abathur config oauth-login --manual
Enter OAuth tokens manually:
Obtain tokens from Claude Code or console.anthropic.com

Access token: ********
Refresh token: ********
Expires in (seconds) [3600]: 3600

✓ OAuth tokens stored in keychain
Expires: 2025-10-09 15:30:00 UTC
```

---

### `oauth-status` - Check Authentication Status

```bash
abathur config oauth-status
```

**Example output (OAuth):**
```
  Authentication Status
┏━━━━━━━━━━━━━━━┳━━━━━━━━━━━━━━━━━━━━┓
┃ Property      ┃ Value              ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━━━━━━━━━━━━━━┩
│ Auth Method   │ OAuth              │
│ Context Limit │ 200,000 tokens     │
│ Token Expiry  │ 2h 15m remaining   │
└───────────────┴────────────────────┘
```

**Example output (No Auth):**
```
  Authentication Status
┏━━━━━━━━━━━━━━━┳━━━━━━━┓
┃ Property      ┃ Value ┃
┡━━━━━━━━━━━━━━━╇━━━━━━━┩
│ Auth Method   │ None  │
│ Context Limit │ N/A   │
│ Token Expiry  │ N/A   │
└───────────────┴───────┘

No authentication configured.
Configure authentication:
  1. API key: abathur config set-key <key>
  2. OAuth:   abathur config oauth-login --manual
```

---

### `oauth-refresh` - Manually Refresh Tokens

```bash
abathur config oauth-refresh
```

**When to use:**
- Token is expired or near expiry
- Testing token refresh functionality
- Troubleshooting authentication issues

**Note:** Token refresh happens automatically during normal usage. Manual refresh is rarely needed.

**Example:**
```bash
$ abathur config oauth-refresh
Refreshing OAuth tokens...
✓ Token refreshed successfully
Expires: 2025-10-09 16:30:00 UTC
```

---

### `oauth-logout` - Clear OAuth Tokens

```bash
abathur config oauth-logout
```

**What it does:**
- Clears tokens from system keychain
- Removes tokens from `.env` file
- Clears environment variables

**Example:**
```bash
$ abathur config oauth-logout
✓ OAuth tokens cleared
```

---

## Environment Variables

### API Key Authentication

```bash
# Set API key
export ANTHROPIC_API_KEY="sk-ant-api03-your-key-here"

# Verify
abathur config oauth-status
```

### OAuth Authentication

```bash
# Set OAuth tokens
export ANTHROPIC_AUTH_TOKEN="your-access-token"
export ANTHROPIC_OAUTH_REFRESH_TOKEN="your-refresh-token"
export ANTHROPIC_OAUTH_EXPIRES_AT="2025-10-09T15:30:00+00:00"

# Verify
abathur config oauth-status
```

**Note:** Environment variables override keychain and `.env` file storage.

---

## Storage Options

### System Keychain (Recommended)

**Advantages:**
- ✅ Encrypted by OS
- ✅ Persistent across sessions
- ✅ Secure credential management

**Platforms:**
- macOS: Keychain Access
- Linux: Secret Service API
- Windows: Windows Credential Manager

**Usage:**
```bash
# Default behavior stores in keychain
abathur config oauth-login --manual
```

---

### `.env` File

**Advantages:**
- ✅ Portable (can be committed to private repos)
- ✅ Works in all environments
- ✅ Easy to backup

**Disadvantages:**
- ⚠️ Less secure (plain text file)
- ⚠️ Must set file permissions to 0600

**Usage:**
```bash
# Store in .env file
abathur config oauth-login --manual --no-use-keychain

# File will be created at: .env
# Permissions automatically set to 0600 (user read/write only)
```

**Example `.env` file:**
```bash
ANTHROPIC_AUTH_TOKEN=your-access-token
ANTHROPIC_OAUTH_REFRESH_TOKEN=your-refresh-token
ANTHROPIC_OAUTH_EXPIRES_AT=2025-10-09T15:30:00+00:00
```

---

## Automatic Token Refresh

Abathur automatically refreshes OAuth tokens using two strategies:

### 1. Proactive Refresh

Tokens are refreshed **5 minutes before expiry** to prevent mid-request failures.

```
Token expires in 6 minutes → Continue using current token
Token expires in 4 minutes → Proactively refresh token
Token expires in 3 minutes → Use refreshed token
```

### 2. Reactive Refresh

If an API request returns **401 Unauthorized**, Abathur:
1. Detects authentication failure
2. Refreshes token using refresh token
3. Retries the request (up to 3 attempts)
4. Returns error if all retries fail

**Retry Logic:**
```
Attempt 1: Request → 401 → Refresh → Retry
Attempt 2: Request → 401 → Refresh → Retry
Attempt 3: Request → 401 → Refresh → Retry
Attempt 4: Give up → Return error
```

---

## Context Window Limits

Abathur automatically detects context window limits based on your authentication method:

| Auth Method | Context Limit | Warning Threshold |
|-------------|---------------|-------------------|
| API Key | 1,000,000 tokens | 900,000 tokens (90%) |
| OAuth | 200,000 tokens | 180,000 tokens (90%) |

### Context Window Warnings

When your input approaches the limit, Abathur logs a warning:

```
WARNING: Task input (185,000 tokens) approaching oauth context limit (200,000 tokens)
Auth method: oauth
Percentage: 92.5%
```

**Solutions:**
1. Use API key authentication for large tasks
2. Reduce input size
3. Split task into smaller subtasks

---

## Switching Between Auth Methods

### To Use API Key

```bash
# Set API key (takes priority over OAuth)
export ANTHROPIC_API_KEY="sk-ant-api03-your-key"

# Run task
abathur task submit large-task
# Uses API key (1M context limit)
```

### To Use OAuth

```bash
# Unset API key
unset ANTHROPIC_API_KEY

# Run task
abathur task submit small-task
# Uses OAuth (200K context limit)
```

### To Force Specific Method

```bash
# Use only API key
abathur config oauth-logout  # Clear OAuth tokens
export ANTHROPIC_API_KEY="sk-ant-api03-your-key"

# Use only OAuth
unset ANTHROPIC_API_KEY  # Remove API key
abathur config oauth-login --manual  # Configure OAuth
```

---

## Security Best Practices

1. **Use Keychain Storage**
   ```bash
   # Default: stores in keychain (encrypted)
   abathur config oauth-login --manual
   ```

2. **Set Proper File Permissions**
   ```bash
   # If using .env file, verify permissions
   ls -l .env
   # Should show: -rw-------  (0600)

   # Fix if needed:
   chmod 600 .env
   ```

3. **Never Commit Tokens to Git**
   ```bash
   # Add .env to .gitignore
   echo ".env" >> .gitignore
   ```

4. **Rotate Tokens Regularly**
   ```bash
   # Log out and log in with fresh tokens
   abathur config oauth-logout
   abathur config oauth-login --manual
   ```

5. **Use Environment-Specific Configurations**
   ```bash
   # Development
   export ANTHROPIC_AUTH_TOKEN="dev-token"

   # Production
   export ANTHROPIC_API_KEY="prod-api-key"
   ```

---

## Troubleshooting

See [OAUTH_TROUBLESHOOTING.md](./OAUTH_TROUBLESHOOTING.md) for common issues and solutions.

---

## Next Steps

- [OAuth Migration Guide](./OAUTH_MIGRATION_GUIDE.md) - Migrating from API key to OAuth
- [OAuth Troubleshooting](./OAUTH_TROUBLESHOOTING.md) - Common issues and solutions
- [Architecture Documentation](../prd_oauth_spawning/04_system_architecture.md) - Technical details

---

**Need help?** Open an issue at [github.com/your-org/abathur/issues](https://github.com/your-org/abathur/issues)
