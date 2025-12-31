/**
 * Rate limiter to prevent overwhelming Alchemy API
 * Implements token bucket algorithm
 */
export class RateLimiter {
  private tokens: number;
  private readonly maxTokens: number;
  private readonly refillRate: number; // tokens per second
  private lastRefill: number;

  constructor(maxTokens = 100, refillRate = 10) {
    this.maxTokens = maxTokens;
    this.tokens = maxTokens;
    this.refillRate = refillRate;
    this.lastRefill = Date.now();
  }

  private refill(): void {
    const now = Date.now();
    const timePassed = (now - this.lastRefill) / 1000; // seconds
    const tokensToAdd = timePassed * this.refillRate;

    this.tokens = Math.min(this.maxTokens, this.tokens + tokensToAdd);
    this.lastRefill = now;
  }

  async waitForToken(): Promise<void> {
    this.refill();

    if (this.tokens >= 1) {
      this.tokens -= 1;
      return;
    }

    // Not enough tokens, wait
    const waitTime = ((1 - this.tokens) / this.refillRate) * 1000;
    await new Promise((resolve) => setTimeout(resolve, waitTime));
    this.tokens = 0;
  }

  async executeWithLimit<T>(fn: () => Promise<T>): Promise<T> {
    await this.waitForToken();
    return fn();
  }
}

/**
 * Exponential backoff for retries
 */
export class ExponentialBackoff {
  private attempt = 0;
  private readonly maxAttempts: number;
  private readonly baseDelay: number;
  private readonly maxDelay: number;

  constructor(maxAttempts = 5, baseDelay = 1000, maxDelay = 30000) {
    this.maxAttempts = maxAttempts;
    this.baseDelay = baseDelay;
    this.maxDelay = maxDelay;
  }

  async execute<T>(fn: () => Promise<T>): Promise<T> {
    while (this.attempt < this.maxAttempts) {
      try {
        const result = await fn();
        this.attempt = 0; // Reset on success
        return result;
      } catch (error) {
        this.attempt++;

        if (this.attempt >= this.maxAttempts) {
          throw new Error(`Max retry attempts (${this.maxAttempts}) exceeded: ${error}`);
        }

        const delay = Math.min(this.baseDelay * Math.pow(2, this.attempt), this.maxDelay);
        console.log(`Retry attempt ${this.attempt}/${this.maxAttempts} after ${delay}ms...`);

        await new Promise((resolve) => setTimeout(resolve, delay));
      }
    }

    throw new Error('Should not reach here');
  }

  reset(): void {
    this.attempt = 0;
  }
}
