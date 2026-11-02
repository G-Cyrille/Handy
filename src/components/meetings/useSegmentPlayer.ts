import { useCallback, useEffect, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { readFile } from "@tauri-apps/plugin-fs";
import type { Segment } from "@/bindings";
import { useOsType } from "@/hooks/useOsType";
import { errorMessage } from "./meetingFormat";

/**
 * One shared audio element for a list of segments, so a page of 100 segments
 * costs one player instead of 100. Linux WebKitGTK can't stream asset URLs,
 * so segments are read into a blob there, like the history view does.
 */
export const useSegmentPlayer = () => {
  const osType = useOsType();
  const audioRef = useRef<HTMLAudioElement | null>(null);
  const blobUrlRef = useRef<string | null>(null);
  const [playingId, setPlayingId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const releaseBlob = () => {
    if (blobUrlRef.current) URL.revokeObjectURL(blobUrlRef.current);
    blobUrlRef.current = null;
  };

  useEffect(
    () => () => {
      audioRef.current?.pause();
      releaseBlob();
    },
    [],
  );

  const toggle = useCallback(
    async (segment: Pick<Segment, "id" | "path">) => {
      const audio = (audioRef.current ??= new Audio());
      if (playingId === segment.id) {
        audio.pause();
        setPlayingId(null);
        return;
      }
      audio.pause();
      releaseBlob();
      setError(null);
      try {
        if (osType === "linux") {
          const bytes = await readFile(segment.path);
          blobUrlRef.current = URL.createObjectURL(
            new Blob([bytes], { type: "audio/wav" }),
          );
          audio.src = blobUrlRef.current;
        } else {
          audio.src = convertFileSrc(segment.path, "asset");
        }
        audio.onended = () => setPlayingId(null);
        setPlayingId(segment.id);
        await audio.play();
      } catch (e) {
        setPlayingId(null);
        setError(errorMessage(e));
      }
    },
    [osType, playingId],
  );

  return { playingId, toggle, error };
};

export type SegmentPlayer = ReturnType<typeof useSegmentPlayer>;
