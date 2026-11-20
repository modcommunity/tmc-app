import { Link, useParams } from 'react-router-dom'

import { BTN_GHOST } from '~/components/play-center'

/**
 * Route and nav slot for the avatar studio (`/avatar`, `/games/:slug/avatar`).
 * The editor is built by gs-app-avatar-1, which replaces this element.
 */
export default function AvatarSlotRoute() {
    const { slug } = useParams()

    return (
        <div className="flex flex-col items-start gap-3 p-4">
            <h1 className="text-lg font-bold">Avatar</h1>
            <p className="text-sm text-muted">
                The avatar studio is coming to the app. Your outfits are listed on your account.
            </p>
            <Link to={slug ? `/games/${slug}` : '/games'} className={BTN_GHOST}>
                Back
            </Link>
        </div>
    )
}
