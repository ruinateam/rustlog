import { useMutation, useQuery } from '@tanstack/vue-query'
import { computed, type MaybeRefOrGetter, toValue } from 'vue'
import { ApiError, api, type BasicMessage, type TierMode, unwrap } from './client'
import type { TimeRange } from '@/lib/periods'

/** Twitch data changes rarely; logs only when a day is not over yet. */
const LOOKUP_STALE_TIME = 10 * 60 * 1000

/** Retries only failures that may go away: not 4xx problems. */
function retry(failureCount: number, error: Error) {
  const temporary = !(error instanceof ApiError) || error.problem.status >= 500
  return temporary && failureCount < 2
}

export function useChannels() {
  return useQuery({
    queryKey: ['channels'],
    queryFn: () => unwrap(api.GET('/channels')),
    select: (data) => data.channels,
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

/** A user as given in a URL: by login, or by id when the login is unknown. */
export type UserRef = { login: string } | { id: string }

/** The id and login of a user. */
export function useUser(ref: MaybeRefOrGetter<UserRef | undefined>) {
  return useQuery({
    queryKey: computed(() => ['user', toValue(ref)]),
    queryFn: async () => {
      const wanted = toValue(ref)!
      const query =
        'login' in wanted ? { login: [wanted.login.toLowerCase()] } : { id: [wanted.id] }
      const { users } = await unwrap(api.GET('/users', { params: { query } }))
      const user = users.find((candidate) =>
        'login' in wanted ? candidate.login === wanted.login.toLowerCase() : candidate.id === wanted.id,
      )
      if (!user) {
        throw new ApiError({
          code: 'not_found',
          status: 404,
          title: 'The requested data was not found',
          detail: 'Twitch does not know the user',
        })
      }
      return user
    },
    enabled: computed(() => Boolean(toValue(ref))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export function useLogDates(channelId: MaybeRefOrGetter<string | undefined>) {
  return useQuery({
    queryKey: computed(() => ['log-dates', toValue(channelId)]),
    queryFn: () =>
      unwrap(
        api.GET('/channels/{channelId}/log-dates', {
          params: { path: { channelId: toValue(channelId)! } },
        }),
      ),
    select: (data) => data.dates,
    enabled: computed(() => Boolean(toValue(channelId))),
    retry,
  })
}

export function useLogMonths(
  channelId: MaybeRefOrGetter<string | undefined>,
  userId: MaybeRefOrGetter<string | undefined>,
) {
  return useQuery({
    queryKey: computed(() => ['log-months', toValue(channelId), toValue(userId)]),
    queryFn: () =>
      unwrap(
        api.GET('/channels/{channelId}/users/{userId}/log-months', {
          params: { path: { channelId: toValue(channelId)!, userId: toValue(userId)! } },
        }),
      ),
    select: (data) => data.months,
    enabled: computed(() => Boolean(toValue(channelId) && toValue(userId))),
    retry,
  })
}

/** Messages of a channel, or of one user in it, in a time range. */
export function useMessages(
  channelId: MaybeRefOrGetter<string | undefined>,
  userId: MaybeRefOrGetter<string | undefined>,
  range: MaybeRefOrGetter<TimeRange | undefined>,
) {
  return useQuery({
    queryKey: computed(() => ['messages', toValue(channelId), toValue(userId), toValue(range)]),
    queryFn: async (): Promise<BasicMessage[]> => {
      const channel = toValue(channelId)!
      const user = toValue(userId)
      const { from, to } = toValue(range)!
      const query = { from, to, format: 'basic-json' as const }
      const data = user
        ? await unwrap(
            api.GET('/channels/{channelId}/users/{userId}/logs', {
              params: { path: { channelId: channel, userId: user }, query },
            }),
          )
        : await unwrap(
            api.GET('/channels/{channelId}/logs', {
              params: { path: { channelId: channel }, query },
            }),
          )
      // `basic-json` answers basic messages.
      return (data as { messages: BasicMessage[] }).messages
    },
    enabled: computed(() => Boolean(toValue(channelId) && toValue(range))),
    retry,
  })
}

export function useChatBadges(channelId: MaybeRefOrGetter<string | undefined>) {
  return useQuery({
    queryKey: computed(() => ['badges', toValue(channelId)]),
    queryFn: () =>
      unwrap(
        api.GET('/channels/{channelId}/badges', {
          params: { path: { channelId: toValue(channelId)! } },
        }),
      ),
    select: (data) => data.badges,
    enabled: computed(() => Boolean(toValue(channelId))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export function useChannelStats(channelId: MaybeRefOrGetter<string | undefined>) {
  return useQuery({
    queryKey: computed(() => ['channel-stats', toValue(channelId)]),
    queryFn: () =>
      unwrap(
        api.GET('/channels/{channelId}/stats', {
          params: { path: { channelId: toValue(channelId)! } },
        }),
      ),
    enabled: computed(() => Boolean(toValue(channelId))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export function useUserStats(
  channelId: MaybeRefOrGetter<string | undefined>,
  userId: MaybeRefOrGetter<string | undefined>,
) {
  return useQuery({
    queryKey: computed(() => ['user-stats', toValue(channelId), toValue(userId)]),
    queryFn: () =>
      unwrap(
        api.GET('/channels/{channelId}/users/{userId}/stats', {
          params: { path: { channelId: toValue(channelId)!, userId: toValue(userId)! } },
        }),
      ),
    enabled: computed(() => Boolean(toValue(channelId) && toValue(userId))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export function useNameHistory(userId: MaybeRefOrGetter<string | undefined>) {
  return useQuery({
    queryKey: computed(() => ['name-history', toValue(userId)]),
    queryFn: () =>
      unwrap(
        api.GET('/users/{userId}/name-history', {
          params: { path: { userId: toValue(userId)! } },
        }),
      ),
    select: (data) => data.names,
    enabled: computed(() => Boolean(toValue(userId))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export interface TierRequest {
  channelId: string
  /** `YYYY-MM-DD`, `YYYY-MM` or `YYYY`. */
  period: string
  mode: TierMode
}

export function useTiers(request: MaybeRefOrGetter<TierRequest | undefined>) {
  return useQuery({
    queryKey: computed(() => ['tiers', toValue(request)]),
    queryFn: () => {
      const { channelId, period, mode } = toValue(request)!
      return unwrap(
        api.GET('/channels/{channelId}/tiers/{period}', {
          params: { path: { channelId, period }, query: { mode } },
        }),
      )
    },
    enabled: computed(() => Boolean(toValue(request))),
    staleTime: LOOKUP_STALE_TIME,
    retry,
  })
}

export function useRandomMessage() {
  return useMutation({
    mutationFn: async ({ channelId, userId }: { channelId: string; userId?: string }) => {
      const data = userId
        ? await unwrap(
            api.GET('/channels/{channelId}/users/{userId}/logs/random', {
              params: { path: { channelId, userId } },
            }),
          )
        : await unwrap(
            api.GET('/channels/{channelId}/logs/random', { params: { path: { channelId } } }),
          )
      return (data as { messages: BasicMessage[] }).messages[0]
    },
  })
}

export function useOptOutCode() {
  return useMutation({
    mutationFn: () => unwrap(api.POST('/opt-out-codes')),
  })
}
