/**
 * Playlists: named groups of tracks the user keeps for themselves.
 *
 * Entries are identities (artist + title), not files. That is the point: a playlist
 * can hold a track that was never saved, and if the same track later lands in the
 * library, playback picks the file automatically because the player looks it up by
 * the same identity.
 */
export type PlaylistTrack = {
  artist: string;
  title: string;
  artwork: string | null;
  duration: number;
  source: string;
};

export type Playlist = {
  id: string;
  name: string;
  tracks: PlaylistTrack[];
};

const LS = "sb.playlists";

export function keyOf(t: { artist: string; title: string }): string {
  return `${t.artist.toLowerCase()}\u0000${t.title.toLowerCase()}`;
}

export function readPlaylists(): Playlist[] {
  try {
    const raw = localStorage.getItem(LS);
    const parsed = raw ? JSON.parse(raw) : null;
    if (Array.isArray(parsed)) {
      return parsed.filter((p) => p && typeof p.name === "string" && Array.isArray(p.tracks));
    }
  } catch {
    /* a corrupt list should not take the app down */
  }
  return [];
}

export function writePlaylists(list: Playlist[]) {
  try {
    localStorage.setItem(LS, JSON.stringify(list));
  } catch {
    /* storage full: the session still works */
  }
}

export function newPlaylist(name: string): Playlist {
  return {
    id: `pl-${Date.now().toString(36)}`,
    name: name.trim() || "Untitled",
    tracks: [],
  };
}

/** The same track twice in one playlist is never useful. */
export function withTrack(list: Playlist[], id: string, track: PlaylistTrack): Playlist[] {
  return list.map((p) =>
    p.id === id && !p.tracks.some((t) => keyOf(t) === keyOf(track))
      ? { ...p, tracks: [...p.tracks, track] }
      : p,
  );
}

export function withoutTrack(list: Playlist[], id: string, track: { artist: string; title: string }): Playlist[] {
  return list.map((p) =>
    p.id === id ? { ...p, tracks: p.tracks.filter((t) => keyOf(t) !== keyOf(track)) } : p,
  );
}

export function renamePlaylist(list: Playlist[], id: string, name: string): Playlist[] {
  return list.map((p) => (p.id === id ? { ...p, name: name.trim() || p.name } : p));
}
