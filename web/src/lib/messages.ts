/** An emote image, from Twitch or a third-party emote service. */
export interface Emote {
  code: string
  url: string
  /** `srcset` with larger versions for high density screens. */
  srcset: string
}

/** A piece of a chat message as it is rendered. */
export type Token =
  | { kind: 'text'; text: string }
  | { kind: 'emote'; emote: Emote }
  | { kind: 'link'; url: string }

/** Where the IRC `emotes` tag places a Twitch emote in the text. */
interface EmoteRange {
  id: string
  /** Index of the first code point. */
  start: number
  /** Index of the last code point, inclusive. */
  end: number
}

const LINK = /^https?:\/\/\S+$/i

/** Parses the IRC `emotes` tag: `id:start-end,start-end/id:start-end`. */
export function parseEmotesTag(tag: string | undefined): EmoteRange[] {
  if (!tag) return []
  const ranges: EmoteRange[] = []
  for (const entry of tag.split('/')) {
    const [id, positions] = entry.split(':')
    if (!id || !positions) continue
    for (const position of positions.split(',')) {
      const [start, end] = position.split('-').map(Number)
      if (Number.isInteger(start) && Number.isInteger(end) && start! <= end!) {
        ranges.push({ id, start: start!, end: end! })
      }
    }
  }
  return ranges.sort((a, b) => a.start - b.start)
}

export function twitchEmote(id: string, code: string): Emote {
  const base = `https://static-cdn.jtvnw.net/emoticons/v2/${id}/default/dark`
  return { code, url: `${base}/1.0`, srcset: `${base}/1.0 1x, ${base}/2.0 2x, ${base}/3.0 4x` }
}

/**
 * Splits a message into text, emotes and links. Twitch emotes come from the
 * `emotes` tag, whose positions count Unicode code points; words matching a
 * third-party emote code become that emote.
 */
export function tokenize(
  text: string,
  emotesTag: string | undefined,
  thirdPartyEmotes: ReadonlyMap<string, Emote>,
): Token[] {
  const codePoints = Array.from(text)
  const tokens: Token[] = []
  let position = 0

  for (const range of parseEmotesTag(emotesTag)) {
    // Overlapping or out of range positions come from bad data; skip them.
    if (range.start < position || range.end >= codePoints.length) continue
    tokens.push(...tokenizeWords(codePoints.slice(position, range.start).join(''), thirdPartyEmotes))
    const code = codePoints.slice(range.start, range.end + 1).join('')
    tokens.push({ kind: 'emote', emote: twitchEmote(range.id, code) })
    position = range.end + 1
  }
  tokens.push(...tokenizeWords(codePoints.slice(position).join(''), thirdPartyEmotes))

  return mergeText(tokens)
}

function tokenizeWords(text: string, thirdPartyEmotes: ReadonlyMap<string, Emote>): Token[] {
  return text.split(/(\s+)/).flatMap((word): Token[] => {
    if (word === '') return []
    const emote = thirdPartyEmotes.get(word)
    if (emote) return [{ kind: 'emote', emote }]
    if (LINK.test(word)) return [{ kind: 'link', url: word }]
    return [{ kind: 'text', text: word }]
  })
}

function mergeText(tokens: Token[]): Token[] {
  const merged: Token[] = []
  for (const token of tokens) {
    const last = merged.at(-1)
    if (token.kind === 'text' && last?.kind === 'text') {
      last.text += token.text
    } else {
      merged.push(token)
    }
  }
  return merged
}

/** The badges of the IRC `badges` tag, as `set/version` keys. */
export function badgeKeys(tag: string | undefined): string[] {
  return tag ? tag.split(',').filter(Boolean) : []
}
