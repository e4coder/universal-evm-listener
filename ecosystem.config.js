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
        SQLITE_PATH: path.join(baseDir, 'data', 'transfers.db'),
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
        SQLITE_PATH: path.join(baseDir, 'data', 'transfers.db'),
      },
      error_file: path.join(baseDir, 'logs', 'api-error.log'),
      out_file: path.join(baseDir, 'logs', 'api-out.log'),
      log_date_format: 'YYYY-MM-DD HH:mm:ss Z',
      merge_logs: true,
      time: true,
    },
  ],
};
