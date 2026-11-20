/**
 * Response shapes for the GameSpice screens that the contract mirror does not
 * carry. `src/lib/api/contract.ts` is a verbatim copy of website-city's file and
 * is never hand-edited; these are the three reads the hub adds, shaped from the
 * routes' own types (`WorkshopMineFor`, `PointsSummary`, `avatar/outfits`).
 * Move them into the contract when website-city exports them there.
 */

import { z } from 'zod'

export const WorkshopMineItemSchema = z.object({
    subscriptionId: z.string(),
    assetId: z.number().int(),
    name: z.string(),
    image: z.string().nullable(),
    webUrl: z.string(),
    scope: z.enum(['all', 'listed']),
    autoUpdate: z.boolean().optional(),
    paused: z.boolean().optional(),
    available: z.boolean().optional(),
    via: z.object({ collectionId: z.number().int(), name: z.string() }).nullable().optional(),
    games: z.array(z.unknown()).optional(),
})

export const WorkshopMineResponseSchema = z.object({
    items: z.array(WorkshopMineItemSchema),
})

export const PointsSummarySchema = z.object({
    site: z.object({
        earned: z.number(),
        spent: z.number(),
        available: z.number(),
    }),
    apps: z.array(
        z.object({
            appId: z.number().int(),
            trusted: z.boolean(),
            earned: z.number(),
            spent: z.number(),
            available: z.number(),
        })
    ),
    total: z.number(),
})

/** A saved outfit; `doc` is the avatar studio's figure document. */
export const OutfitSchema = z.object({
    id: z.union([z.string(), z.number()]),
    name: z.string(),
    doc: z.unknown().optional(),
    savedAt: z.union([z.string(), z.number()]).optional(),
})

export const OutfitsResponseSchema = z.object({
    outfits: z.array(OutfitSchema),
})

export type WorkshopMineItemT = z.infer<typeof WorkshopMineItemSchema>
export type PointsSummaryT = z.infer<typeof PointsSummarySchema>
export type OutfitT = z.infer<typeof OutfitSchema>
