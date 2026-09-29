import { describe, expect, it } from 'vitest'
import { badgeKeys, type Emote, parseEmotesTag, tokenize } from './messages'

const pepe: Emote = { code: 'PepeLaugh', url: 'https://cdn/pepe', srcset: '' }
const thirdParty = new Map([[pepe.code, pepe]])

describe('parseEmotesTag', () => {
  it('reads several emotes with several positions', () => {
    expect(parseEmotesTag('25:0-4,12-16/1902:6-10')).toEqual([
      { id: '25', start: 0, end: 4 },
      { id: '1902', start: 6, end: 10 },
      { id: '25', start: 12, end: 16 },
    ])
  })

  it('ignores empty and malformed tags', () => {
    expect(parseEmotesTag(undefined)).toEqual([])
    expect(parseEmotesTag('')).toEqual([])
    expect(parseEmotesTag('25:x-4/:1-2/7:5-3')).toEqual([])
  })
})

describe('tokenize', () => {
  it('replaces Twitch emotes by position', () => {
    const tokens = tokenize('Kappa hi Kappa', '25:0-4,9-13', new Map())
    expect(tokens.map((token) => token.kind)).toEqual(['emote', 'text', 'emote'])
    expect(tokens[1]).toEqual({ kind: 'text', text: ' hi ' })
    expect(tokens[0]).toMatchObject({ emote: { code: 'Kappa' } })
  })

  it('counts positions in code points, not UTF-16 units', () => {
    const tokens = tokenize('😀 Kappa', '25:2-6', new Map())
    expect(tokens).toMatchObject([
      { kind: 'text', text: '😀 ' },
      { kind: 'emote', emote: { code: 'Kappa' } },
    ])
  })

  it('replaces whole words that are third-party emotes', () => {
    expect(tokenize('so PepeLaugh here', undefined, thirdParty)).toEqual([
      { kind: 'text', text: 'so ' },
      { kind: 'emote', emote: pepe },
      { kind: 'text', text: ' here' },
    ])
    expect(tokenize('PepeLaughing', undefined, thirdParty)).toEqual([
      { kind: 'text', text: 'PepeLaughing' },
    ])
  })

  it('finds links', () => {
    expect(tokenize('see https://example.com/a?b=1 now', undefined, new Map())).toEqual([
      { kind: 'text', text: 'see ' },
      { kind: 'link', url: 'https://example.com/a?b=1' },
      { kind: 'text', text: ' now' },
    ])
  })

  it('skips emote positions outside the text', () => {
    expect(tokenize('hi', '25:0-4', new Map())).toEqual([{ kind: 'text', text: 'hi' }])
  })
})

describe('badgeKeys', () => {
  it('splits the badges tag', () => {
    expect(badgeKeys('subscriber/12,premium/1')).toEqual(['subscriber/12', 'premium/1'])
    expect(badgeKeys('')).toEqual([])
    expect(badgeKeys(undefined)).toEqual([])
  })
})
