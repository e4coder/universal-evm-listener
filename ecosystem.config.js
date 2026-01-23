const path = require('path');
const baseDir = __dirname;

module.exports = {
  apps: [
    {
      name: 'rust-listener',
      script: './rust-listener/target/release/rust-listener',
      cwd: path.join(baseDir, 'rust-listener'),
      instances: 1,
      exec_mode: 'fork',
      autorestart: true,
      watch: false,
      max_memory_restart: '500M',
      env: {
        ALCHEMY_API_KEY: process.env.ALCHEMY_API_KEY,
        DATABASE_URL: process.env.DATABASE_URL || 'postgres://erc20cache:erc20cache_pass@localhost:5433/erc20cache',
        TTL_SECS: '600',
        LOG_LEVEL: 'info',
      },
      error_file: path.join(baseDir, 'logs', 'rust-listener-error.log'),
      out_file: path.join(baseDir, 'logs', 'rust-listener-out.log'),
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      merge_logs: true,
      time: true,
    },
    {
      name: 'blockchain-api',
      script: 'dist/api/server.js',
      cwd: baseDir,
      instances: 1,
      exec_mode: 'fork',
      autorestart: true,
      watch: false,
      max_memory_restart: '500M',
      env: {
        NODE_ENV: 'production',
        API_PORT: 5459,
        DATABASE_URL: process.env.DATABASE_URL || 'postgres://erc20cache:erc20cache_pass@localhost:5433/erc20cache',
      },
      error_file: path.join(baseDir, 'logs', 'api-error.log'),
      out_file: path.join(baseDir, 'logs', 'api-out.log'),
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      merge_logs: true,
      time: true,
    },
  ],
};
