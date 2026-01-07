/**
 * Event monitoring and metrics tracking
 * Helps detect anomalies and missing events
 */
export class EventMonitor {
  private metrics: {
    [chainId: number]: {
      totalEvents: number;
      erc20Events: number;
      nativeEvents: number;
      blocksProcessed: number;
      missedBlocks: number;
      backfillEvents: number;
      lastEventTime: number;
      reconnections: number;
      errors: number;
    };
  } = {};

  constructor() {}

  private getMetrics(chainId: number) {
    if (!this.metrics[chainId]) {
      this.metrics[chainId] = {
        totalEvents: 0,
        erc20Events: 0,
        nativeEvents: 0,
        blocksProcessed: 0,
        missedBlocks: 0,
        backfillEvents: 0,
        lastEventTime: Date.now(),
        reconnections: 0,
        errors: 0,
      };
    }
    return this.metrics[chainId];
  }

  recordERC20Event(chainId: number, isBackfill = false): void {
    const m = this.getMetrics(chainId);
    m.totalEvents++;
    m.erc20Events++;
    if (isBackfill) m.backfillEvents++;
    m.lastEventTime = Date.now();
  }

  recordNativeEvent(chainId: number, isBackfill = false): void {
    const m = this.getMetrics(chainId);
    m.totalEvents++;
    m.nativeEvents++;
    if (isBackfill) m.backfillEvents++;
    m.lastEventTime = Date.now();
  }

  recordBlockProcessed(chainId: number): void {
    this.getMetrics(chainId).blocksProcessed++;
  }

  recordMissedBlocks(chainId: number, count: number): void {
    const m = this.getMetrics(chainId);
    m.missedBlocks += count;

    // Alert if too many blocks missed
    if (count > 100) {
      console.warn(
        `⚠️  [Chain ${chainId}] HIGH ALERT: ${count} blocks missed! This may indicate a serious issue.`
      );
    }
  }

  recordReconnection(chainId: number): void {
    this.getMetrics(chainId).reconnections++;
  }

  recordError(chainId: number): void {
    this.getMetrics(chainId).errors++;
  }

  getStats(chainId: number): any {
    return this.getMetrics(chainId);
  }

  getAllStats(): any {
    return this.metrics;
  }

  /**
   * Check for anomalies
   */
  checkHealth(chainId: number): {
    healthy: boolean;
    issues: string[];
  } {
    const m = this.getMetrics(chainId);
    const issues: string[] = [];

    // Check if no events in last 10 minutes (for active chains)
    const timeSinceLastEvent = Date.now() - m.lastEventTime;
    if (timeSinceLastEvent > 10 * 60 * 1000 && m.totalEvents > 0) {
      issues.push(`No events for ${Math.floor(timeSinceLastEvent / 60000)} minutes`);
    }

    // Check if too many reconnections
    if (m.reconnections > 10) {
      issues.push(`Excessive reconnections (${m.reconnections})`);
    }

    // Check if too many errors
    if (m.errors > 50) {
      issues.push(`High error count (${m.errors})`);
    }

    // Check if too many missed blocks
    if (m.missedBlocks > 1000) {
      issues.push(`Many missed blocks (${m.missedBlocks})`);
    }

    return {
      healthy: issues.length === 0,
      issues,
    };
  }

  /**
   * Print summary stats
   */
  printSummary(chainId: number, networkName: string): void {
    const m = this.getMetrics(chainId);
    console.log(`\n📊 [${networkName}] Statistics:`);
    console.log(`   Total Events: ${m.totalEvents}`);
    console.log(`   - ERC20: ${m.erc20Events}`);
    console.log(`   - Native: ${m.nativeEvents}`);
    console.log(`   Blocks Processed: ${m.blocksProcessed}`);
    console.log(`   Missed Blocks: ${m.missedBlocks}`);
    console.log(`   Backfilled Events: ${m.backfillEvents}`);
    console.log(`   Reconnections: ${m.reconnections}`);
    console.log(`   Errors: ${m.errors}`);

    const health = this.checkHealth(chainId);
    if (health.healthy) {
      console.log(`   Status: ✅ Healthy`);
    } else {
      console.log(`   Status: ⚠️  Issues detected:`);
      health.issues.forEach((issue) => console.log(`     - ${issue}`));
    }
  }

  /**
   * Start periodic health checks
   */
  startHealthChecks(intervalMs = 300000): void {
    // Every 5 minutes
    setInterval(() => {
      console.log('\n🏥 Health Check Report:');
      for (const chainId in this.metrics) {
        const health = this.checkHealth(parseInt(chainId));
        if (!health.healthy) {
          console.log(`\n⚠️  Chain ${chainId} has issues:`);
          health.issues.forEach((issue) => console.log(`   - ${issue}`));
        }
      }
    }, intervalMs);
  }
}
