import { useQuery } from '@tanstack/vue-query'
import { computed, type ComputedRef, type MaybeRefOrGetter, toValue } from 'vue'
import type { Emote } from '@/lib/messages'

/** Emote sets change rarely; fetch each at most once an hour. */
const STALE_TIME = 60 * 60 * 1000

interface SevenTvEmote {
  name: string
  data: { host: { url: string; files: { name: string; format: string }[] } }
}

interface BttvEmote {
  id: string
  code: string
}

interface FfzSet {
  emoticons: { name: string; urls: Record<string, string> }[]
}

async function fetchJson<T>(url: string): Promise<T> {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`${url}: ${response.status}`)
  return response.json() as Promise<T>
}

function sevenTv(emotes: SevenTvEmote[] | undefined): Emote[] {
  return (emotes ?? []).flatMap((emote) => {
    const files = emote.data.host.files.filter((file) => file.format === 'WEBP')
    // 7TV gives protocol-relative URLs.
    const host = emote.data.host.url
    const base = host.startsWith('//') ? `https:${host}` : host
    const [small, medium, large] = files
    if (!small) return []
    return [
      {
        code: emote.name,
        url: `${base}/${small.name}`,
        srcset: [small, medium, large]
          .flatMap((file, index) => (file ? [`${base}/${file.name} ${index + 1}x`] : []))
          .join(', '),
      },
    ]
  })
}

function bttv(emotes: BttvEmote[]): Emote[] {
  return emotes.map(({ id, code }) => {
    const base = `https://cdn.betterttv.net/emote/${id}`
    return { code, url: `${base}/1x`, srcset: `${base}/1x 1x, ${base}/2x 2x, ${base}/3x 3x` }
  })
}

function ffz(sets: Record<string, FfzSet> | undefined): Emote[] {
  return Object.values(sets ?? {}).flatMap((set) =>
    set.emoticons.map(({ name, urls }) => ({
      code: name,
      url: urls['1'] ?? '',
      srcset: Object.entries(urls)
        .map(([scale, url]) => `${url} ${scale}x`)
        .join(', '),
    })),
  )
}

/** A list of emotes from one source; failures count as no emotes. */
function useEmoteSource(
  key: unknown[] | ComputedRef<unknown[]>,
  load: () => Promise<Emote[]>,
  enabled = computed(() => true),
) {
  return useQuery({
    queryKey: key,
    queryFn: () => load().catch(() => [] as Emote[]),
    staleTime: STALE_TIME,
    enabled,
  })
}

/**
 * The 7TV, BTTV and FFZ emotes usable in a channel, by code. Channel emotes
 * win over global ones of the same code.
 */
export function useThirdPartyEmotes(channelId: MaybeRefOrGetter<string | undefined>) {
  const hasChannel = computed(() => Boolean(toValue(channelId)))
  const channel = () => toValue(channelId)!

  const sources = [
    useEmoteSource(['7tv', 'global'], async () =>
      sevenTv((await fetchJson<{ emotes: SevenTvEmote[] }>('https://7tv.io/v3/emote-sets/global')).emotes),
    ),
    useEmoteSource(['bttv', 'global'], async () =>
      bttv(await fetchJson<BttvEmote[]>('https://api.betterttv.net/3/cached/emotes/global')),
    ),
    useEmoteSource(['ffz', 'global'], async () =>
      ffz((await fetchJson<{ sets: Record<string, FfzSet> }>('https://api.frankerfacez.com/v1/set/global')).sets),
    ),
    useEmoteSource(
      computed(() => ['7tv', toValue(channelId)]),
      async () =>
        sevenTv(
          (await fetchJson<{ emote_set?: { emotes: SevenTvEmote[] } }>(`https://7tv.io/v3/users/twitch/${channel()}`))
            .emote_set?.emotes,
        ),
      hasChannel,
    ),
    useEmoteSource(
      computed(() => ['bttv', toValue(channelId)]),
      async () => {
        const data = await fetchJson<{ channelEmotes: BttvEmote[]; sharedEmotes: BttvEmote[] }>(
          `https://api.betterttv.net/3/cached/users/twitch/${channel()}`,
        )
        return bttv([...data.channelEmotes, ...data.sharedEmotes])
      },
      hasChannel,
    ),
    useEmoteSource(
      computed(() => ['ffz', toValue(channelId)]),
      async () =>
        ffz((await fetchJson<{ sets: Record<string, FfzSet> }>(`https://api.frankerfacez.com/v1/room/id/${channel()}`)).sets),
      hasChannel,
    ),
  ]

  return computed(() => {
    const byCode = new Map<string, Emote>()
    for (const source of sources) {
      for (const emote of source.data.value ?? []) byCode.set(emote.code, emote)
    }
    return byCode
  })
}
