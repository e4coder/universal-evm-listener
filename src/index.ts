/**
 * Universal Blockchain Listener - API Server Entry Point
 *
 * NOTE: The blockchain listener has been rewritten in Rust for better performance.
 * This Node.js application now only serves the API.
 *
 * To run the listener: ./rust-listener/target/release/rust-listener
 * To run the API: npm run api
 *
 * The legacy Node.js listener code is available in the legacy/ folder.
 */

console.log('');
console.log('='.repeat(60));
console.log('  Universal Blockchain Listener');
console.log('='.repeat(60));
console.log('');
console.log('  The blockchain listener has been rewritten in Rust.');
console.log('  This Node.js package now only provides the API server.');
console.log('');
console.log('  To start:');
console.log('    1. Run Rust listener: ./rust-listener/target/release/rust-listener');
console.log('    2. Run API server:    npm run api');
console.log('');
console.log('  Or use PM2:');
console.log('    pm2 start ecosystem.config.js');
console.log('');
console.log('='.repeat(60));
console.log('');

// Re-export the query service for programmatic use
export { QueryService } from './services/queryService';
export { SQLiteCache } from './cache/sqlite';
