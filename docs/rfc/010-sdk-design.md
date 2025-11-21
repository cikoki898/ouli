# RFC-010: SDK Design

**Status**: 🔵 Draft  
**Author**: System  
**Created**: 2025-11-20

## Abstract

Define language-specific SDK designs (Rust, TypeScript, Python) that provide ergonomic APIs for managing Ouli proxy lifecycle, configuration, and testing workflows.

## Design Goals

1. **Zero Configuration**: Sensible defaults for 90% use cases
2. **Type Safety**: Leverage language type systems
3. **Async/Await**: Native async support where available
4. **Process Management**: Handle binary lifecycle automatically
5. **Test Framework Integration**: Jest, pytest, Rust tests
6. **Migration Path**: Easy upgrade from test-server

## Rust SDK

### Core API

```rust
// lib.rs
pub struct Ouli {
    config: Config,
    process: Option<Child>,
    mode: Mode,
}

impl Ouli {
    pub fn builder() -> OuliBuilder {
        OuliBuilder::default()
    }
    
    pub async fn start(&mut self) -> Result<()> {
        assert!(self.process.is_none(), "Already started");
        
        // Spawn Ouli binary
        let mut cmd = Command::new("ouli");
        cmd.arg(match self.mode {
            Mode::Record => "record",
            Mode::Replay => "replay",
        });
        cmd.arg("--config").arg(&self.config.path);
        
        self.process = Some(cmd.spawn()?);
        
        // Wait for health check
        self.wait_for_ready().await?;
        
        Ok(())
    }
    
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut process) = self.process.take() {
            process.kill()?;
            process.wait()?;
        }
        Ok(())
    }
    
    async fn wait_for_ready(&self) -> Result<()> {
        let health_url = format!(
            "http://localhost:{}/health",
            self.config.endpoints[0].source_port
        );
        
        for attempt in 0..30 {
            if reqwest::get(&health_url).await.is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        Err(OuliError::StartupTimeout)
    }
}

impl Drop for Ouli {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
        }
    }
}
```

### Builder Pattern

```rust
pub struct OuliBuilder {
    mode: Mode,
    recording_dir: PathBuf,
    endpoints: Vec<EndpointConfig>,
    redaction: RedactionConfig,
}

impl OuliBuilder {
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = mode;
        self
    }
    
    pub fn recording_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.recording_dir = dir.into();
        self
    }
    
    pub fn endpoint(mut self, endpoint: EndpointConfig) -> Self {
        self.endpoints.push(endpoint);
        self
    }
    
    pub fn redact_secret(mut self, secret: impl Into<String>) -> Self {
        self.redaction.secrets.push(secret.into());
        self
    }
    
    pub async fn build(self) -> Result<Ouli> {
        // Generate config file
        let config_path = self.recording_dir.join("ouli-config.toml");
        let config = Config {
            mode: self.mode,
            recording_dir: self.recording_dir.clone(),
            endpoints: self.endpoints,
            redaction: self.redaction,
            ..Default::default()
        };
        
        std::fs::write(&config_path, toml::to_string(&config)?)?;
        
        Ok(Ouli {
            config: OuliConfig {
                path: config_path,
            },
            process: None,
            mode: self.mode,
        })
    }
}
```

### Test Integration

```rust
// Test helper macros
#[macro_export]
macro_rules! ouli_test {
    ($name:ident, $body:expr) => {
        #[tokio::test]
        async fn $name() {
            let mut ouli = Ouli::builder()
                .mode(Mode::Replay)
                .recording_dir(format!("./recordings/{}", stringify!($name)))
                .endpoint(EndpointConfig {
                    target_host: "api.example.com".into(),
                    target_port: 443,
                    source_port: 8080,
                    ..Default::default()
                })
                .build()
                .await
                .unwrap();
            
            ouli.start().await.unwrap();
            
            $body.await;
            
            ouli.stop().await.unwrap();
        }
    };
}

// Usage
ouli_test!(test_api_call, async {
    let client = reqwest::Client::new();
    let response = client
        .get("http://localhost:8080/api/users")
        .send()
        .await
        .unwrap();
    
    assert_eq!(response.status(), 200);
});
```

## TypeScript SDK

### Core API

```typescript
// index.ts
export class Ouli {
    private config: OuliConfig;
    private process?: ChildProcess;
    private mode: 'record' | 'replay';
    
    constructor(config: OuliConfig) {
        this.config = config;
        this.mode = config.mode;
    }
    
    async start(): Promise<void> {
        if (this.process) {
            throw new Error('Ouli already started');
        }
        
        // Find binary
        const binaryPath = await this.findBinary();
        
        // Spawn process
        this.process = spawn(binaryPath, [
            this.mode,
            '--config', this.config.configPath,
        ]);
        
        // Setup logging
        this.process.stdout?.on('data', (data) => {
            console.log(`[ouli] ${data}`);
        });
        
        this.process.stderr?.on('data', (data) => {
            console.error(`[ouli] ${data}`);
        });
        
        // Wait for ready
        await this.waitForReady();
    }
    
    async stop(): Promise<void> {
        if (this.process) {
            this.process.kill('SIGTERM');
            await new Promise((resolve) => {
                this.process!.once('exit', resolve);
                setTimeout(() => {
                    this.process?.kill('SIGKILL');
                    resolve(undefined);
                }, 5000);
            });
            this.process = undefined;
        }
    }
    
    private async waitForReady(): Promise<void> {
        const healthUrl = `http://localhost:${this.config.endpoints[0].sourcePort}/health`;
        
        for (let i = 0; i < 30; i++) {
            try {
                await fetch(healthUrl);
                return;
            } catch {
                await new Promise(r => setTimeout(r, 100));
            }
        }
        
        throw new Error('Ouli failed to start');
    }
    
    private async findBinary(): Promise<string> {
        // Check if bundled binary exists
        const bundledPath = path.join(__dirname, '../bin/ouli');
        if (fs.existsSync(bundledPath)) {
            return bundledPath;
        }
        
        // Check PATH
        return 'ouli';
    }
}

// Builder
export class OuliBuilder {
    private config: Partial<OuliConfig> = {
        endpoints: [],
        redaction: { secrets: [], regexPatterns: [] },
    };
    
    mode(mode: 'record' | 'replay'): this {
        this.config.mode = mode;
        return this;
    }
    
    recordingDir(dir: string): this {
        this.config.recordingDir = dir;
        return this;
    }
    
    endpoint(endpoint: EndpointConfig): this {
        this.config.endpoints!.push(endpoint);
        return this;
    }
    
    redactSecret(secret: string): this {
        this.config.redaction!.secrets!.push(secret);
        return this;
    }
    
    async build(): Promise<Ouli> {
        // Validate
        if (!this.config.mode) {
            throw new Error('Mode is required');
        }
        if (!this.config.recordingDir) {
            throw new Error('Recording directory is required');
        }
        
        // Generate config file
        const configPath = path.join(this.config.recordingDir, 'ouli-config.toml');
        await fs.promises.mkdir(this.config.recordingDir, { recursive: true });
        await fs.promises.writeFile(configPath, this.generateConfig());
        
        return new Ouli({
            ...this.config as OuliConfig,
            configPath,
        });
    }
    
    private generateConfig(): string {
        return TOML.stringify(this.config);
    }
}
```

### Jest Integration

```typescript
// jest-preset.ts
import { OuliBuilder } from './index';

let ouli: Ouli | undefined;

export default {
    globalSetup: async () => {
        // Determine mode from environment
        const mode = process.argv.includes('--record') ? 'record' : 'replay';
        
        ouli = await new OuliBuilder()
            .mode(mode)
            .recordingDir('./recordings')
            .endpoint({
                targetHost: 'api.example.com',
                targetPort: 443,
                sourcePort: 8080,
                targetType: 'https',
                sourceType: 'http',
            })
            .redactSecret(process.env.API_KEY || '')
            .build();
        
        await ouli.start();
    },
    
    globalTeardown: async () => {
        if (ouli) {
            await ouli.stop();
        }
    },
};

// Usage in tests
describe('API Tests', () => {
    test('fetch users', async () => {
        const response = await fetch('http://localhost:8080/api/users', {
            headers: {
                'x-ouli-test-name': 'fetch_users',
            },
        });
        
        expect(response.status).toBe(200);
        const data = await response.json();
        expect(data.users).toBeDefined();
    });
});
```

## Python SDK

### Core API

```python
# ouli/__init__.py
from typing import Optional, List
from dataclasses import dataclass
import subprocess
import time
import requests

@dataclass
class EndpointConfig:
    target_host: str
    target_port: int
    source_port: int
    target_type: str = "https"
    source_type: str = "http"
    redact_headers: List[str] = None

class Ouli:
    def __init__(self, config: 'OuliConfig'):
        self.config = config
        self.process: Optional[subprocess.Popen] = None
        
    def start(self):
        """Start the Ouli proxy"""
        if self.process:
            raise RuntimeError("Ouli already started")
        
        # Spawn process
        cmd = [
            'ouli',
            self.config.mode,
            '--config', self.config.config_path,
        ]
        
        self.process = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        
        # Wait for ready
        self._wait_for_ready()
        
    def stop(self):
        """Stop the Ouli proxy"""
        if self.process:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
            self.process = None
    
    def _wait_for_ready(self, timeout: int = 30):
        """Wait for proxy to be ready"""
        health_url = f"http://localhost:{self.config.endpoints[0].source_port}/health"
        
        for _ in range(timeout * 10):
            try:
                requests.get(health_url, timeout=0.5)
                return
            except:
                time.sleep(0.1)
        
        raise TimeoutError("Ouli failed to start")
    
    def __enter__(self):
        self.start()
        return self
    
    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()

class OuliBuilder:
    def __init__(self):
        self._mode: Optional[str] = None
        self._recording_dir: Optional[str] = None
        self._endpoints: List[EndpointConfig] = []
        self._secrets: List[str] = []
    
    def mode(self, mode: str) -> 'OuliBuilder':
        self._mode = mode
        return self
    
    def recording_dir(self, dir: str) -> 'OuliBuilder':
        self._recording_dir = dir
        return self
    
    def endpoint(self, endpoint: EndpointConfig) -> 'OuliBuilder':
        self._endpoints.append(endpoint)
        return self
    
    def redact_secret(self, secret: str) -> 'OuliBuilder':
        self._secrets.append(secret)
        return self
    
    def build(self) -> Ouli:
        if not self._mode:
            raise ValueError("Mode is required")
        if not self._recording_dir:
            raise ValueError("Recording directory is required")
        
        # Generate config
        import os
        import toml
        
        os.makedirs(self._recording_dir, exist_ok=True)
        config_path = os.path.join(self._recording_dir, 'ouli-config.toml')
        
        config_data = {
            'mode': self._mode,
            'recording_dir': self._recording_dir,
            'endpoints': [e.__dict__ for e in self._endpoints],
            'redaction': {
                'secrets': self._secrets,
            },
        }
        
        with open(config_path, 'w') as f:
            toml.dump(config_data, f)
        
        return Ouli(OuliConfig(
            mode=self._mode,
            recording_dir=self._recording_dir,
            endpoints=self._endpoints,
            config_path=config_path,
        ))
```

### pytest Integration

```python
# conftest.py
import pytest
from ouli import OuliBuilder, EndpointConfig

@pytest.fixture(scope="session")
def ouli():
    """Global Ouli instance for all tests"""
    mode = 'record' if '--record' in sys.argv else 'replay'
    
    ouli = OuliBuilder() \
        .mode(mode) \
        .recording_dir('./recordings') \
        .endpoint(EndpointConfig(
            target_host='api.example.com',
            target_port=443,
            source_port=8080,
        )) \
        .redact_secret(os.environ.get('API_KEY', '')) \
        .build()
    
    ouli.start()
    yield ouli
    ouli.stop()

# Usage in tests
def test_fetch_users(ouli):
    import requests
    
    response = requests.get(
        'http://localhost:8080/api/users',
        headers={'x-ouli-test-name': 'fetch_users'}
    )
    
    assert response.status_code == 200
    data = response.json()
    assert 'users' in data
```

## CLI Wrapper

```rust
// src/main.rs
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ouli")]
#[command(about = "Deterministic HTTP/WebSocket record-replay proxy")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    /// Path to config file
    #[arg(short, long, default_value = "ouli-config.toml")]
    config: PathBuf,
    
    /// Log level
    #[arg(short, long, default_value = "info")]
    log_level: String,
}

#[derive(Subcommand)]
enum Commands {
    /// Record HTTP traffic
    Record {
        /// Directory to store recordings
        #[arg(short, long, default_value = "./recordings")]
        recording_dir: PathBuf,
    },
    
    /// Replay recorded traffic
    Replay {
        /// Directory containing recordings
        #[arg(short, long, default_value = "./recordings")]
        recording_dir: PathBuf,
    },
    
    /// Show statistics
    Stats {
        /// Recording directory to analyze
        recording_dir: PathBuf,
    },
    
    /// Migrate from test-server
    Migrate {
        /// test-server recordings directory
        from: PathBuf,
        /// Ouli recordings directory
        to: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    // Setup logging
    tracing_subscriber::fmt()
        .with_env_filter(cli.log_level)
        .init();
    
    match cli.command {
        Commands::Record { recording_dir } => {
            let mut config = Config::from_file(&cli.config)?;
            config.mode = Mode::Record;
            config.recording_dir = recording_dir;
            
            let engine = RecordingEngine::new(config).await?;
            engine.run().await?;
        }
        Commands::Replay { recording_dir } => {
            let mut config = Config::from_file(&cli.config)?;
            config.mode = Mode::Replay;
            config.recording_dir = recording_dir;
            
            let engine = ReplayEngine::new(config).await?;
            engine.run().await?;
        }
        Commands::Stats { recording_dir } => {
            show_stats(&recording_dir)?;
        }
        Commands::Migrate { from, to } => {
            migrate_recordings(&from, &to).await?;
        }
    }
    
    Ok(())
}
```

## Package Distribution

### npm Package

```json
{
  "name": "@ouli/sdk",
  "version": "0.1.0",
  "description": "TypeScript SDK for Ouli proxy",
  "main": "dist/index.js",
  "types": "dist/index.d.ts",
  "bin": {
    "ouli": "bin/ouli"
  },
  "scripts": {
    "postinstall": "node scripts/download-binary.js",
    "build": "tsc",
    "test": "jest"
  },
  "files": [
    "dist",
    "bin",
    "scripts"
  ]
}
```

### PyPI Package

```python
# setup.py
from setuptools import setup, find_packages

setup(
    name='ouli-sdk',
    version='0.1.0',
    description='Python SDK for Ouli proxy',
    packages=find_packages(),
    install_requires=[
        'requests>=2.28',
        'toml>=0.10',
    ],
    extras_require={
        'dev': ['pytest>=7.0', 'pytest-asyncio>=0.21'],
    },
    entry_points={
        'console_scripts': [
            'ouli=ouli.cli:main',
        ],
    },
)
```

### cargo Package

```toml
[package]
name = "ouli-sdk"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1.35", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
toml = "0.8"

[dev-dependencies]
tokio-test = "0.4"
```

## Documentation

Each SDK includes:

- **README.md**: Quick start guide
- **API Reference**: Auto-generated docs
- **Examples**: Common use cases
- **Migration Guide**: From test-server

## References

- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [TypeScript Handbook](https://www.typescriptlang.org/docs/handbook/intro.html)
- [Python Packaging Guide](https://packaging.python.org/)
