// The Drachm mark: a Greek key (meander) folded into a D, with a red centre.
export function Coin({ size = 22 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" fill="none" aria-hidden="true" className="drh-mark">
      <path
        d="M12 54 V10 H42 L54 22 V42 L42 54 H22 V20 H40 V44 H31 V31"
        stroke="currentColor"
        strokeWidth="5"
        strokeLinejoin="miter"
        strokeLinecap="square"
      />
      <rect x="28.5" y="28.5" width="5" height="5" fill="var(--drh)" />
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
