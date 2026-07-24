import type { SVGProps } from "react";

/** Inline Heroicons (MIT, solid 24x24) used as small colored status icons so state is never
 *  carried by color alone. Each fills with `currentColor`; size and color come from CSS on the
 *  caller (see `.prow__status-icon` / `.panel__summary-icon` in Panel.css). */
function statusIcon(path: string) {
  return function StatusIcon(props: SVGProps<SVGSVGElement>) {
    return (
      <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
        <path fillRule="evenodd" d={path} clipRule="evenodd" />
      </svg>
    );
  };
}

/** Success: pipeline settled cleanly. */
export const CheckCircleIcon = statusIcon(
  "M2.25 12c0-5.385 4.365-9.75 9.75-9.75s9.75 4.365 9.75 9.75-4.365 9.75-9.75 9.75S2.25 17.385 2.25 12Zm13.36-1.814a.75.75 0 1 0-1.22-.872l-3.236 4.53L9.53 12.22a.75.75 0 0 0-1.06 1.06l2.25 2.25a.75.75 0 0 0 1.14-.094l3.75-5.25Z",
);

/** Failed: pipeline or job failure. */
export const XCircleIcon = statusIcon(
  "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25Zm-1.72 6.97a.75.75 0 1 0-1.06 1.06L10.94 12l-1.72 1.72a.75.75 0 1 0 1.06 1.06L12 13.06l1.72 1.72a.75.75 0 1 0 1.06-1.06L13.06 12l1.72-1.72a.75.75 0 1 0-1.06-1.06L12 10.94l-1.72-1.72Z",
);

/** Pending/manual: waiting to run. */
export const ClockIcon = statusIcon(
  "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25ZM12.75 6a.75.75 0 0 0-1.5 0v6c0 .414.336.75.75.75h4.5a.75.75 0 0 0 0-1.5h-3.75V6Z",
);

/** Running: in progress. Caller applies the spin animation. */
export const ArrowPathIcon = statusIcon(
  "M4.755 10.059a7.5 7.5 0 0 1 12.548-3.364l1.903 1.903h-3.183a.75.75 0 1 0 0 1.5h4.992a.75.75 0 0 0 .75-.75V4.356a.75.75 0 0 0-1.5 0v3.18l-1.9-1.9A9 9 0 0 0 3.306 9.67a.75.75 0 1 0 1.45.388Zm15.408 3.352a.75.75 0 0 0-.919.53 7.5 7.5 0 0 1-12.548 3.364l-1.902-1.903h3.183a.75.75 0 0 0 0-1.5H2.984a.75.75 0 0 0-.75.75v4.992a.75.75 0 0 0 1.5 0v-3.18l1.9 1.9a9 9 0 0 0 15.059-4.035.75.75 0 0 0-.53-.918Z",
);

/** Auth failed: the stored token is dead, distinct from a pipeline failure. */
export const LockClosedIcon = statusIcon(
  "M12 1.5a5.25 5.25 0 0 0-5.25 5.25v3a3 3 0 0 0-3 3v6.75a3 3 0 0 0 3 3h10.5a3 3 0 0 0 3-3v-6.75a3 3 0 0 0-3-3v-3c0-2.9-2.35-5.25-5.25-5.25Zm3.75 8.25v-3a3.75 3.75 0 1 0-7.5 0v3h7.5Z",
);

/** Stale/offline: the server is unreachable, distinct from "not checked yet". */
export const SignalSlashIcon = statusIcon(
  "M2.47 2.47a.75.75 0 0 1 1.06 0l8.407 8.407a1.125 1.125 0 0 1 1.186 1.186l1.462 1.461a3.001 3.001 0 0 0-.464-3.645.75.75 0 1 1 1.061-1.061 4.501 4.501 0 0 1 .486 5.79l1.072 1.072a6.001 6.001 0 0 0-.497-7.923.75.75 0 0 1 1.06-1.06 7.501 7.501 0 0 1 .505 10.05l1.064 1.065a9 9 0 0 0-.508-12.176.75.75 0 0 1 1.06-1.06c3.923 3.922 4.093 10.175.512 14.3l1.594 1.594a.75.75 0 1 1-1.06 1.06l-2.106-2.105-2.121-2.122h-.001l-4.705-4.706a.747.747 0 0 1-.127-.126L2.47 3.53a.75.75 0 0 1 0-1.061Zm1.189 4.422a.75.75 0 0 1 .326 1.01 9.004 9.004 0 0 0 1.651 10.462.75.75 0 1 1-1.06 1.06C1.27 16.12.63 11.165 2.648 7.219a.75.75 0 0 1 1.01-.326ZM5.84 9.134a.75.75 0 0 1 .472.95 6 6 0 0 0 1.444 6.159.75.75 0 0 1-1.06 1.06A7.5 7.5 0 0 1 4.89 9.606a.75.75 0 0 1 .95-.472Zm2.341 2.653a.75.75 0 0 1 .848.638c.088.62.37 1.218.849 1.696a.75.75 0 0 1-1.061 1.061 4.483 4.483 0 0 1-1.273-2.546.75.75 0 0 1 .637-.848Z",
);

/** Unknown: first poll in flight, or no CI configured at all. */
export const MinusCircleIcon = statusIcon(
  "M12 2.25c-5.385 0-9.75 4.365-9.75 9.75s4.365 9.75 9.75 9.75 9.75-4.365 9.75-9.75S17.385 2.25 12 2.25Zm3 10.5a.75.75 0 0 0 0-1.5H9a.75.75 0 0 0 0 1.5h6Z",
);
