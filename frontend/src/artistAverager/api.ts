import type { Artist, Track } from 'src/types';
import { API_BASE_URL } from '../conf';
import { AutocompleteSuggestion } from './ArtistInput/AutocompleteDropdown';

export const getArtistAutocompleteSuggestions = (
  query: string
): Promise<AutocompleteSuggestion[]> => {
  const url = `${API_BASE_URL}/search_artist?q=${encodeURIComponent(query)}`;
  return fetch(url).then((res) => res.json());
};

export interface AverageArtistItem {
  artist: Artist;
  topTracks: Track[];
  similarityToTargetPoint: number;
  similarityToArtist1: number;
  similarityToArtist2: number;
}

export interface AverageArtistsResponse {
  artists: AverageArtistItem[];
  similarity: number;
  distance: number;
}

export const getAverageArtists = (
  artist1SpotifyID: string,
  artist2SpotifyID: string
): Promise<AverageArtistsResponse> =>
  fetch(`${API_BASE_URL}/average_artists/${artist1SpotifyID}/${artist2SpotifyID}`).then((res) =>
    res.json()
  );
