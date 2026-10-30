import type { ProfileView } from "@/bridge";

type SidebarProps = {
  profiles: ProfileView[];
  selected: string;
  activeProgram: string;
  hookEnabled: boolean;
  onSelect: (id: string) => void;
  onAdd: () => void;
};

export function Sidebar({
  profiles,
  selected,
  activeProgram,
  hookEnabled,
  onSelect,
  onAdd,
}: SidebarProps) {
  const liveProfile = profiles.find(
    (profile) =>
      !profile.global &&
      profile.active &&
      profile.program.toLowerCase() === activeProgram.toLowerCase(),
  );

  return (
    <aside className="hb__aside">
      {/* The app's own name and version live in the window header now: this
          rail is only about the profiles the remapper runs. */}
      <div className="hb__rail-label">Profiles</div>

      <nav className="hb__nav" aria-label="Profiles">
        {profiles.map((profile) => {
          const live = liveProfile?.id === profile.id && hookEnabled;
          return (
            <button
              key={profile.id}
              type="button"
              className="hb__profile"
              aria-current={profile.id === selected ? "true" : undefined}
              onClick={() => onSelect(profile.id)}
            >
              <span
                className={`hb__gear-dot${live ? "" : " hb__gear-dot--off"}`}
                aria-hidden="true"
                title={live ? "Live for the program in front" : "Not the active program"}
              />
              <span className="hb__profile-name">
                {profile.global ? "Global" : profile.program}
              </span>
              <span className="hb__profile-meta">
                {profile.mappings.length > 0 ? profile.mappings.length : "—"}
              </span>
            </button>
          );
        })}
      </nav>

      <div className="hb__rail-foot">
        <button type="button" className="hb__add" onClick={onAdd}>
          <svg width="11" height="11" viewBox="0 0 16 16" fill="none" aria-hidden="true">
            <path
              d="M8 3.5v9M3.5 8h9"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
          </svg>
          Add program
        </button>
      </div>
    </aside>
  );
}
