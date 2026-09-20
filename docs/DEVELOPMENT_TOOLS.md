# Development Tools Reference

A comprehensive guide to AI and non-AI CLI tools for code quality, security, and review.

---

## 🤖 AI Tools

### Multi-Model AI Platforms

Tools that provide access to multiple AI models from a single CLI.

#### **LLM CLI**

- **Install**: `npm install -g llm-cli`
- **Purpose**: Unified interface for GPT-4, Claude, Gemini, Llama
- **Usage**:

  ```bash
  llm ask claude "review this code"
  llm ask gpt4 "review this code"
  llm ask gemini "review this code"
  ```

- **Why**: Access multiple AI models without switching tools

#### **AI CLI**

- **Install**: `npm install -g @literally-anything/ai`
- **Purpose**: Multi-provider AI interface
- **Usage**:

  ```bash
  ai chat --provider openai "review code"
  ai chat --provider anthropic "review code"
  ```

- **Why**: Simplified multi-provider access

---

### Individual AI Model CLIs

Direct access to specific AI models.

#### **Gemini CLI** ✅ (Currently Using)

- **Install**: Check Google Cloud SDK
- **Purpose**: Google's Gemini AI for code review
- **Usage**:

  ```bash
  git diff --cached | gemini "Review this code change"
  ```

- **Why**: Strong at code analysis, fast responses

#### **Claude CLI** (Anthropic)

- **Install**: `pip install anthropic-cli`
- **Purpose**: Access Claude models (Sonnet 4.5, Opus 4.5)
- **Usage**:

  ```bash
  anthropic chat --model claude-sonnet-4.5 "Review this code"
  cat file.rs | anthropic chat "Review this Rust code"
  ```

- **Why**: Same model powering Claude Code, excellent reasoning

#### **OpenAI CLI** (GPT-4)

- **Install**: `pip install openai`
- **Purpose**: Access GPT-4 and other OpenAI models
- **Usage**:

  ```bash
  openai api chat.completions.create \
    --model gpt-4 \
    --message "Review: $(git diff --cached)"
  ```

- **Why**: Industry-leading code understanding

#### **Ollama** ⭐ (Local LLMs)

- **Install**: `brew install ollama`
- **Purpose**: Run AI models locally (privacy, no API costs)
- **Popular Models**:
  - `deepseek-coder` - Excellent for code review
  - `codellama` - Meta's code-focused model
  - `qwen2.5-coder` - Strong coding capabilities
  - `starcoder2` - Code generation specialist
- **Usage**:

  ```bash
  # Pull a model
  ollama pull deepseek-coder:33b

  # Use for reviews
  ollama run codellama "review this code"
  git diff | ollama run deepseek-coder "Review these changes"
  ```

- **Why**: Privacy (runs locally), free, fast, offline capable

---

### Specialized AI Code Tools

AI tools specifically designed for coding tasks.

#### **CodeRabbit CLI** ✅ (Currently Using)

- **Install**: `brew install coderabbit`
- **Purpose**: Specialized AI code review tool
- **Usage**:

  ```bash
  coderabbit review
  ```

- **Why**: Purpose-built for code review, line-by-line analysis

#### **Aider** ⭐ (AI Pair Programmer)

- **Install**: `pip install aider-chat`
- **Purpose**: AI that can directly edit code files
- **Usage**:

  ```bash
  # Interactive mode
  aider --model claude-sonnet-4.5
  aider --model gpt-4
  aider --model ollama/deepseek-coder

  # Review mode
  aider --review
  ```

- **Why**: Can auto-fix issues, not just review

#### **Mentat**

- **Install**: `pip install mentat`
- **Purpose**: AI coding assistant for code changes
- **Usage**:

  ```bash
  mentat
  ```

- **Why**: Context-aware code modifications

#### **GPT-Engineer**

- **Install**: `pip install gpt-engineer`
- **Purpose**: AI-driven feature development
- **Usage**:

  ```bash
  gpt-engineer
  ```

- **Why**: Generates entire features from descriptions

#### **Tabby** (Self-hosted)

- **Install**: `docker run -it tabbyml/tabby`
- **Purpose**: Self-hosted AI coding assistant
- **Why**: Full control, privacy, no external dependencies

---

## 🔒 Security Scanning Tools

Non-AI tools for security analysis and vulnerability detection.

### **Snyk CLI** ⭐ (Highly Recommended)

- **Install**: `brew install snyk/tap/snyk`
- **Purpose**: Dependency vulnerabilities, code security, IaC scanning
- **Usage**:

  ```bash
  snyk test                    # Scan dependencies
  snyk code test               # Scan source code (SAST)
  snyk container test myimage  # Scan containers
  snyk iac test                # Scan infrastructure as code
  ```

- **Why**: Comprehensive security coverage, great free tier

### **GitGuardian** (Secret Scanning)

- **Install**: `brew install gitguardian/tap/ggshield`
- **Purpose**: Detect secrets, API keys, credentials in code
- **Usage**:

  ```bash
  ggshield secret scan          # Scan current directory
  ggshield secret scan --pre-commit  # Use as pre-commit hook
  ```

- **Why**: Prevents credential leaks, real-time detection

### **Semgrep** (SAST)

- **Install**: `brew install semgrep`
- **Purpose**: Static application security testing
- **Usage**:

  ```bash
  semgrep scan --config=auto
  semgrep scan --config=p/owasp-top-ten
  semgrep scan --config=p/security-audit
  ```

- **Why**: Fast, customizable, catches code vulnerabilities

### **TruffleHog** (Secret Detection)

- **Install**: `brew install trufflesecurity/trufflehog/trufflehog`
- **Purpose**: Find secrets in git history
- **Usage**:

  ```bash
  trufflehog git file://. --only-verified
  trufflehog github --org=myorg
  ```

- **Why**: Deep history scanning, finds old secrets

### **Trivy** (Vulnerability Scanner)

- **Install**: `brew install aquasecurity/trivy/trivy`
- **Purpose**: All-in-one vulnerability scanner
- **Usage**:

  ```bash
  trivy fs .                    # Scan filesystem
  trivy image myimage:latest    # Scan container image
  trivy config .                # Scan IaC configs
  trivy fs . --security-checks vuln,config,secret
  ```

- **Why**: Multi-purpose, fast, comprehensive

---

## 📊 Code Quality & Linting Tools

### **Prettier** (Code Formatting)

- **Install**: `npm install -g prettier`
- **Purpose**: Opinionated code formatter
- **Usage**:

  ```bash
  prettier --check .
  prettier --write .
  ```

- **Why**: Consistent code style, auto-fixes

### **ESLint** (JavaScript/TypeScript)

- **Install**: `npm install -g eslint`
- **Purpose**: JavaScript/TypeScript linting
- **Usage**:

  ```bash
  eslint .
  eslint --fix .
  ```

- **Why**: Catch bugs, enforce style

### **Clippy** ✅ (Rust - Currently Using)

- **Install**: Included with Rust
- **Purpose**: Rust linter
- **Usage**:

  ```bash
  cargo clippy
  ```

- **Why**: Catches Rust-specific issues

### **Commitlint** (Commit Messages)

- **Install**: `npm install -g @commitlint/cli @commitlint/config-conventional`
- **Purpose**: Validate conventional commit messages
- **Usage**:

  ```bash
  echo "feat: add feature" | commitlint
  ```

- **Why**: Enforce commit message standards

### **SonarLint/SonarScanner**

- **Install**: Via SonarQube setup
- **Purpose**: Continuous code quality inspection
- **Usage**:

  ```bash
  sonar-scanner
  ```

- **Why**: Comprehensive quality metrics

---

## 🧪 Testing & Performance Tools

### **Lighthouse CI** (Web Performance)

- **Install**: `npm install -g @lhci/cli`
- **Purpose**: Performance, accessibility, SEO testing
- **Usage**:

  ```bash
  lhci autorun
  lhci collect
  ```

- **Why**: Web performance insights

### **k6** (Load Testing)

- **Install**: `brew install k6`
- **Purpose**: Performance and load testing
- **Usage**:

  ```bash
  k6 run script.js
  ```

- **Why**: API performance testing

### **Playwright** (E2E Testing)

- **Install**: `npm install -g @playwright/test`
- **Purpose**: End-to-end browser testing
- **Usage**:

  ```bash
  playwright test
  ```

- **Why**: Reliable E2E tests

---

## 📝 Documentation & Utility Tools

### **Tree** (File Structure)

- **Install**: `brew install tree`
- **Purpose**: Display directory structure
- **Usage**:

  ```bash
  tree -L 2 -I 'node_modules|dist'
  ```

- **Why**: Visualize project structure

### **Doctoc** (Table of Contents)

- **Install**: `npm install -g doctoc`
- **Purpose**: Auto-generate markdown TOCs
- **Usage**:

  ```bash
  doctoc README.md
  ```

- **Why**: Keep docs organized

### **jq** (JSON Processing)

- **Install**: `brew install jq`
- **Purpose**: Parse and manipulate JSON
- **Usage**:

  ```bash
  cat data.json | jq '.field'
  ```

- **Why**: Essential for JSON manipulation

---

## 🎯 Recommended Toolchain by Use Case

### **For Pre-Commit Reviews** ⭐

```bash
# Quality
nx-run -t lint,build,test

# Security
ggshield secret scan
snyk test

# AI Reviews
ollama run deepseek-coder    # Fast, local
gemini                        # Cloud
coderabbit review            # Specialized
```

### **For Production Deployments**

```bash
# Security deep scan
snyk test
semgrep scan --config=p/security-audit
trivy fs . --security-checks vuln,config,secret

# Performance
lhci autorun

# AI final review
aider --model claude-sonnet-4.5 --review
```

### **For Daily Development**

```bash
# Quick local AI review
ollama run deepseek-coder

# Format & lint
prettier --write .
eslint --fix .
cargo clippy --fix
```

---

## 📦 Quick Installation Scripts

### AI Tools - Option A (Privacy + Local)

```bash
# Local AI (free, private)
brew install ollama
ollama pull deepseek-coder:33b
ollama pull codellama:34b
```

### AI Tools - Option B (Cloud + Quality)

```bash
# Cloud AI (API keys required)
pip install anthropic-cli openai aider-chat
npm install -g llm-cli
```

### Security Tools (Essential)

```bash
brew install snyk/tap/snyk
brew install gitguardian/tap/ggshield
brew install semgrep
brew install aquasecurity/trivy/trivy
```

### All-in-One Setup

```bash
# AI
brew install ollama
pip install aider-chat anthropic-cli

# Security
brew install snyk/tap/snyk gitguardian/tap/ggshield semgrep

# Quality
npm install -g prettier eslint @commitlint/cli

# Utilities
brew install tree jq
```

---

## 🔗 Integration with Claude Code

All these tools can be integrated into Claude Code custom commands. See `.claude/commands/` for examples.

**Current integrations:**

- ✅ `nx-run` (quality checks)
- ✅ `gemini` (AI review)
- ✅ `coderabbit` (AI review)
- ✅ `git` commands (version control)

**Recommended additions:**

- `ollama` (local AI reviews)
- `ggshield` (secret scanning)
- `snyk` (security scanning)
- `aider` (AI-assisted fixes)

See `.claude/settings.json` for tool permissions.

---

## 📚 Additional Resources

- [Claude Code Documentation](https://code.claude.com/docs)
- [Ollama Model Library](https://ollama.com/library)
- [Snyk Documentation](https://docs.snyk.io)
- [Semgrep Rules](https://semgrep.dev/r)
- [OWASP Top 10](https://owasp.org/www-project-top-ten/)

---

**Last Updated**: 2025-12-08
**Maintained By**: Development Team
