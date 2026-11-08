import type { JSX } from "preact";

export type IconName =
  | "chat"
  | "layers"
  | "sliders"
  | "search"
  | "plus"
  | "back"
  | "more"
  | "attach"
  | "mic"
  | "send"
  | "check"
  | "cpu"
  | "image"
  | "spark"
  | "drive"
  | "shield"
  | "sun"
  | "info"
  | "chevron"
  | "bolt"
  | "close"
  | "download"
  | "play"
  | "stop";

const paths: Record<IconName, JSX.Element> = {
  chat: (
    <>
      <path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 16.5H9l-4 3.5v-3.5H5A1.5 1.5 0 0 1 3.5 15V7A1.5 1.5 0 0 1 5 5.5Z" />
      <path d="M8 9.5h8M8 12.5h5" />
    </>
  ),
  layers: (
    <>
      <path d="m12 3 8.5 4.5L12 12 3.5 7.5 12 3Z" />
      <path d="m3.5 12 8.5 4.5 8.5-4.5M3.5 16.5 12 21l8.5-4.5" />
    </>
  ),
  sliders: (
    <>
      <path d="M4 7h9M17 7h3M4 12h3M11 12h9M4 17h11M19 17h1" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="12" r="2" />
      <circle cx="17" cy="17" r="2" />
    </>
  ),
  search: (
    <>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  back: <path d="M14.5 5 8 12l6.5 7" />,
  more: (
    <>
      <circle cx="5.5" cy="12" r="1" fill="currentColor" stroke="none" />
      <circle cx="12" cy="12" r="1" fill="currentColor" stroke="none" />
      <circle cx="18.5" cy="12" r="1" fill="currentColor" stroke="none" />
    </>
  ),
  attach: <path d="m9 12.5 5.7-5.7a3 3 0 1 1 4.2 4.2l-7.3 7.3a5 5 0 0 1-7.1-7.1l7.3-7.3" />,
  mic: (
    <>
      <rect x="9" y="3.5" width="6" height="11" rx="3" />
      <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v2.5" />
    </>
  ),
  send: <path d="M12 19V5M6.5 10.5 12 5l5.5 5.5" />,
  check: <path d="m5 12.5 4.5 4.5L19 7" />,
  cpu: (
    <>
      <rect x="7" y="7" width="10" height="10" rx="2" />
      <path d="M10 3.5v3M14 3.5v3M10 17.5v3M14 17.5v3M3.5 10h3M3.5 14h3M17.5 10h3M17.5 14h3" />
    </>
  ),
  image: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
      <circle cx="9" cy="10" r="1.6" />
      <path d="m4.5 17 5-4.5 3.5 3 2.5-2 4 3.5" />
    </>
  ),
  spark: (
    <>
      <path d="M12 3.5c.7 3.9 2.6 5.8 6.5 6.5-3.9.7-5.8 2.6-6.5 6.5-.7-3.9-2.6-5.8-6.5-6.5 3.9-.7 5.8-2.6 6.5-6.5Z" />
      <path d="M18.5 15.5c.3 1.7 1.1 2.5 2.8 2.8-1.7.3-2.5 1.1-2.8 2.8-.3-1.7-1.1-2.5-2.8-2.8 1.7-.3 2.5-1.1 2.8-2.8Z" />
    </>
  ),
  drive: (
    <>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
      <path d="M3.5 13.5h17" />
      <circle cx="7" cy="16.5" r=".9" fill="currentColor" stroke="none" />
    </>
  ),
  shield: <path d="M12 3.5 19 6v5.2c0 4-2.8 7.3-7 9.3-4.2-2-7-5.3-7-9.3V6l7-2.5Z" />,
  sun: (
    <>
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M18.7 5.3l-1.4 1.4M6.7 17.3l-1.4 1.4" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 11v5M12 8.2v.1" />
    </>
  ),
  chevron: <path d="m9.5 6 6 6-6 6" />,
  bolt: <path d="M13 3 5.5 13.5H11l-1 7.5 8-11H12l1-7Z" />,
  close: <path d="m6 6 12 12M18 6 6 18" />,
  download: <path d="M12 3.5v11M7.5 10 12 14.5 16.5 10M4.5 19.5h15" />,
  play: <path d="M8 5.5 19 12 8 18.5v-13Z" />,
  stop: <rect x="6.5" y="6.5" width="11" height="11" rx="2.2" fill="currentColor" stroke="none" />,
};

export function Icon({
  name,
  size = 20,
  class: className,
}: {
  name: IconName;
  size?: number;
  class?: string;
}) {
  return (
    <svg
      class={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.7"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {paths[name]}
    </svg>
  );
}
