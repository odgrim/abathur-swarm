# OAuth Troubleshooting Guide

Common issues and solutions for OAuth authentication in Abathur.

---

## Authentication Errors

### "OAuth tokens not found"

**Error:**
```
ValueError: OAuth tokens not found. Please authenticate with: abathur config oauth-login
```

**Solutions:**

1. **Configure OAuth tokens:**
   ```bash
   abathur config oauth-login --manual
   ```

2. **Check token storage:**
   ```bash
   # Verify tokens are stored
   abathur config oauth-status
   ```

3. **Check .env file (if not using keychain):**
   ```bash
   cat .env | grep ANTHROPIC_AUTH_TOKEN
   ```

---

### "Token expired and refresh failed"

**Error:**
```
OAuthRefreshError: Token expired and refresh failed

Remediation: Re-authenticate with: abathur config oauth-login
```

**Cause:** Refresh token has expired or been revoked.

**Solution:**
```bash
# Clear old tokens
abathur config oauth-logout

# Login with fresh tokens
abathur config oauth-login --manual
```

---

### "401 Unauthorized" Errors

**Error:**
```
Authentication failed: 401 Unauthorized
```

**Solutions:**

1. **Check token expiry:**
   ```bash
   abathur config oauth-status
   # Look for "Expired" in Token Expiry
   ```

2. **Manually refresh token:**
   ```bash
   abathur config oauth-refresh
   ```

3. **Re-authenticate if refresh fails:**
   ```bash
   abathur config oauth-logout
   abathur config oauth-login --manual
   ```

---

### "Invalid API key format"

**Error:**
```
APIKeyInvalidError: Invalid API key format (must start with sk-ant-api): test-key...
```

**Cause:** API key doesn't match expected format.

**Solution:**
```bash
# Verify API key format (should start with sk-ant-api)
echo $ANTHROPIC_API_KEY

# Set correct API key
abathur config set-key sk-ant-api03-your-actual-key
```

---

## Token Refresh Issues

### Refresh Token Expired

**Symptoms:**
- OAuth refresh fails with 401
- Manual refresh command fails
- Logs show "refresh_token_expired"

**Solution:**
```bash
# Refresh tokens expire after ~90 days
# Re-authenticate to get fresh tokens
abathur config oauth-logout
abathur config oauth-login --manual
```

---

### Network Errors During Refresh

**Error:**
```
Token refresh failed: Connection timeout
```

**Solutions:**

1. **Check internet connection:**
   ```bash
   curl -I https://console.anthropic.com
   ```

2. **Check firewall settings:** Ensure `console.anthropic.com` is accessible

3. **Retry refresh:**
   ```bash
   abathur config oauth-refresh
   ```

---

### Rate Limited During Refresh

**Error:**
```
Token refresh rate limited (429)
```

**Solution:**
- Wait for the retry-after period (shown in logs)
- Abathur automatically retries with exponential backoff
- If persists, wait 5-10 minutes before trying again

---

## Context Window Issues

### "Context window exceeded" Warning

**Warning:**
```
WARNING: Task input (210,000 tokens) exceeds oauth context limit (200,000 tokens)
```

**Solutions:**

1. **Switch to API key (1M token limit):**
   ```bash
   abathur config set-key sk-ant-api03-your-key
   ```

2. **Reduce input size:**
   - Shorten system prompts
   - Remove unnecessary context
   - Split task into smaller subtasks

3. **Check token estimation:**
   ```python
   # Estimate: ~4 characters = 1 token
   input_chars = len(system_prompt) + len(user_message)
   estimated_tokens = input_chars // 4
   ```

---

## Storage Issues

### Keychain Access Denied

**Error:**
```
Failed to store OAuth tokens in keychain: Permission denied
```

**Solutions:**

1. **Use .env file instead:**
   ```bash
   abathur config oauth-login --manual --no-use-keychain
   ```

2. **Grant keychain access (macOS):**
   - Open "Keychain Access" app
   - Allow access for "abathur" application

3. **Check keychain permissions (Linux):**
   ```bash
   # Ensure Secret Service is running
   systemctl status gnome-keyring-daemon
   ```

---

### .env File Permission Errors

**Error:**
```
Permission denied: .env
```

**Solution:**
```bash
# Fix file permissions
chmod 600 .env

# Verify
ls -l .env
# Should show: -rw-------
```

---

### Tokens Not Persisting

**Symptoms:**
- Tokens work during session but disappear after restart
- Need to re-login every time

**Solutions:**

1. **Verify storage method:**
   ```bash
   abathur config oauth-status
   # Check which storage is being used
   ```

2. **Check .env file exists:**
   ```bash
   ls -la .env
   ```

3. **Verify keychain entry (macOS):**
   ```bash
   security find-generic-password -s "abathur" -a "anthropic_oauth_access_token"
   ```

4. **Use explicit storage:**
   ```bash
   # Force .env file storage
   abathur config oauth-login --manual --no-use-keychain
   ```

---

## Environment Variable Issues

### Environment Variables Not Loaded

**Symptoms:**
- `.env` file exists but auth fails
- Environment variables not recognized

**Solutions:**

1. **Check .env file location:**
   ```bash
   # Must be in project root
   pwd
   ls -la .env
   ```

2. **Manually source .env (temporary):**
   ```bash
   export $(cat .env | xargs)
   abathur config oauth-status
   ```

3. **Verify variable names:**
   ```bash
   # Correct names:
   echo $ANTHROPIC_AUTH_TOKEN
   echo $ANTHROPIC_OAUTH_REFRESH_TOKEN
   echo $ANTHROPIC_OAUTH_EXPIRES_AT
   ```

---

### Priority Conflicts

**Symptoms:**
- OAuth configured but API key is used
- Wrong auth method selected

**Explanation:**
API key takes priority over OAuth when both are configured.

**Solutions:**

1. **To use OAuth, unset API key:**
   ```bash
   unset ANTHROPIC_API_KEY
   abathur config oauth-status
   ```

2. **To use API key, clear OAuth:**
   ```bash
   abathur config oauth-logout
   export ANTHROPIC_API_KEY="sk-ant-api03-..."
   ```

---

## CLI Command Issues

### "Interactive OAuth flow not yet implemented"

**Error:**
```
Interactive OAuth flow not yet implemented.
Use --manual flag to enter tokens manually
```

**Solution:**
```bash
# Always use --manual flag (interactive flow not available)
abathur config oauth-login --manual
```

---

### Command Not Found

**Error:**
```
abathur: command not found
```

**Solutions:**

1. **Install Abathur:**
   ```bash
   poetry install
   ```

2. **Use poetry run:**
   ```bash
   poetry run abathur config oauth-status
   ```

3. **Activate virtual environment:**
   ```bash
   poetry shell
   abathur config oauth-status
   ```

---

## Performance Issues

### Slow Authentication

**Symptoms:**
- Auth takes >5 seconds
- Delays before task execution

**Causes:**
- Token refresh happening on every request
- Network latency to refresh endpoint

**Solutions:**

1. **Check token expiry:**
   ```bash
   abathur config oauth-status
   # Ensure token has >5 minutes remaining
   ```

2. **Manually refresh if near expiry:**
   ```bash
   abathur config oauth-refresh
   ```

3. **Switch to API key for performance-critical tasks:**
   ```bash
   abathur config set-key sk-ant-api03-your-key
   ```

---

## Debugging Tips

### Enable Verbose Logging

```bash
# Set log level to DEBUG
export ABATHUR_LOG_LEVEL=DEBUG

# Run command
abathur config oauth-status

# Check logs
tail -f .abathur/logs/abathur.log
```

### Check Structured Logs

Look for these log events:
- `auth_initialized` - Auth method selected
- `oauth_token_refreshed` - Token refresh succeeded
- `oauth_token_refresh_failed` - Token refresh failed
- `context_window_warning` - Input approaching limit
- `auth_failed_attempting_refresh` - 401 retry triggered

### Test Token Refresh

```bash
# Force token refresh
abathur config oauth-refresh

# Check logs for errors
grep "refresh" .abathur/logs/abathur.log
```

### Verify Token Format

```bash
# Access token should be a long string
echo $ANTHROPIC_AUTH_TOKEN | wc -c
# Should be >50 characters

# Refresh token format
echo $ANTHROPIC_OAUTH_REFRESH_TOKEN | wc -c
# Should be >50 characters

# Expires at should be ISO 8601 format
echo $ANTHROPIC_OAUTH_EXPIRES_AT
# Should match: 2025-10-09T15:30:00+00:00
```

---

## Getting Help

### Check Status

```bash
# Verify current configuration
abathur config oauth-status

# Show full config
abathur config show
```

### Collect Diagnostic Info

```bash
# System info
uname -a

# Python version
python --version

# Abathur version
abathur version

# Auth status
abathur config oauth-status

# Recent logs
tail -n 50 .abathur/logs/abathur.log
```

### Report an Issue

Include this information when reporting issues:
1. Error message (full stack trace)
2. Command that failed
3. Auth status output
4. Recent log entries
5. Operating system and Python version

---

## Common Workflow Issues

### "No authentication configured" on Startup

**Error:**
```
No authentication configured. Options:
  1. Set API key: abathur config set-key <key>
  2. Login with OAuth: abathur config oauth-login
```

**Solution:**
```bash
# Choose one:

# Option 1: API Key
abathur config set-key sk-ant-api03-your-key

# Option 2: OAuth
abathur config oauth-login --manual
```

---

### Tokens Work Locally But Not in CI/CD

**Solutions:**

1. **Use environment variables in CI:**
   ```yaml
   env:
     ANTHROPIC_AUTH_TOKEN: ${{ secrets.OAUTH_ACCESS_TOKEN }}
     ANTHROPIC_OAUTH_REFRESH_TOKEN: ${{ secrets.OAUTH_REFRESH_TOKEN }}
     ANTHROPIC_OAUTH_EXPIRES_AT: ${{ secrets.OAUTH_EXPIRES_AT }}
   ```

2. **Or use API key (recommended for CI):**
   ```yaml
   env:
     ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
   ```

---

## Still Having Issues?

1. **Check documentation:**
   - [OAuth Setup Guide](./OAUTH_SETUP_GUIDE.md)
   - [Migration Guide](./OAUTH_MIGRATION_GUIDE.md)

2. **Search existing issues:**
   - [GitHub Issues](https://github.com/your-org/abathur/issues)

3. **Open a new issue:**
   - Provide diagnostic info
   - Include error messages
   - Describe steps to reproduce

---

**Last Updated:** October 9, 2025
