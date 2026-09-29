import createClient from 'openapi-fetch'
import type { components, paths } from './schema'

export type Schemas = components['schemas']
export type ApiProblem = Schemas['ApiProblem']
export type BasicMessage = Schemas['BasicMessage']
export type TierMode = Schemas['TierMode']

/** The v2 API of the backend that serves this page. */
export const api = createClient<paths>({ baseUrl: '/api/v2' })

/** A failed API request, with the problem the backend answered. */
export class ApiError extends Error {
  constructor(readonly problem: ApiProblem) {
    super(problem.detail ?? problem.title)
  }

  get code() {
    return this.problem.code
  }
}

/** The data of an API response, or its problem thrown as an [ApiError]. */
export async function unwrap<T>(
  request: Promise<{ data?: T; error?: unknown; response: Response }>,
): Promise<T> {
  const { data, error, response } = await request
  if (error !== undefined || data === undefined) {
    throw new ApiError(isProblem(error) ? error : unknownProblem(response))
  }
  return data
}

function isProblem(value: unknown): value is ApiProblem {
  return typeof value === 'object' && value !== null && 'code' in value && 'status' in value
}

function unknownProblem(response: Response): ApiProblem {
  return {
    code: 'internal_error',
    status: response.status,
    title: response.statusText || 'The request failed',
  }
}
