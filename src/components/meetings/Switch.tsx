import React from "react";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
}

/** Bare switch, same look as ToggleSwitch, for use inside table rows. */
export const Switch: React.FC<SwitchProps> = ({
  checked,
  onChange,
  disabled = false,
  label,
}) => (
  <label
    className={`flex items-center shrink-0 ${disabled ? "cursor-not-allowed" : "cursor-pointer"}`}
  >
    <input
      type="checkbox"
      aria-label={label}
      className="sr-only peer"
      checked={checked}
      disabled={disabled}
      onChange={(e) => onChange(e.target.checked)}
    />
    <div className="relative w-9 h-5 bg-mid-gray/20 rounded-full peer peer-focus-visible:ring-2 peer-focus-visible:ring-logo-primary peer-checked:after:translate-x-4 rtl:peer-checked:after:-translate-x-4 after:content-[''] after:absolute after:top-[2px] after:start-[2px] after:bg-white after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-background-ui peer-disabled:opacity-50"></div>
  </label>
);
