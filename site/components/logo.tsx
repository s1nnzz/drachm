// A drachm coin: a struck rim, an inner ring and a stamped "D".
export function Coin({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" fill="none" aria-hidden="true" className="drh-coin">
      <circle cx="16" cy="16" r="14.5" stroke="currentColor" strokeWidth="1.5" />
      <circle cx="16" cy="16" r="11" stroke="currentColor" strokeWidth="0.9" strokeDasharray="1.2 2.1" opacity="0.7" />
      <path
        d="M11.5 9.5h4.2c4.1 0 6.3 2.5 6.3 6.5s-2.2 6.5-6.3 6.5h-4.2z"
        fill="currentColor"
      />
      <path d="M14 12.3v7.4" stroke="var(--coin-bg, #fff)" strokeWidth="1.4" strokeLinecap="round" />
    </svg>
  );
}

export function Wordmark() {
  return (
    <span className="inline-flex items-center gap-2 font-semibold tracking-tight">
      <Coin />
      <span>Drachm</span>
    </span>
  );
}
