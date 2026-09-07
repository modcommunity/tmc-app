import { openUrl } from '@tauri-apps/plugin-opener'
import { Button } from '@modcommunity/shared'

import { useApiEnv, webUrl } from '~/lib/api/env'
import { useAuth } from '~/lib/auth/provider'
import { useSettings } from '~/lib/settings/provider'
import { Row, Section, Toggle } from '~/components/form'
import type { UserSettingsT } from '~/lib/api/contract'

/**
 * Account settings — the user's, stored on the website and shared with every
 * device they sign in on.
 *
 * The pane says so out loud. A settings screen that does not distinguish
 * "this device" from "your account" leaves people unable to predict what
 * changing something will do, which is how a notification preference gets
 * turned off on a phone and silently applied to a desktop.
 */

const NOTIFY_ROWS: { key: keyof UserSettingsT; label: string; hint?: string }[] = [
    { key: 'emailNotifications', label: 'Email notifications' },
    { key: 'pushNotifications', label: 'Push notifications' },
    { key: 'notifyComments', label: 'Comments' },
    { key: 'notifyReviews', label: 'Reviews' },
    { key: 'notifyReleases', label: 'New releases' },
    { key: 'notifyMentions', label: 'Mentions' },
    { key: 'notifyMessages', label: 'Direct messages' },
    { key: 'notifyFriends', label: 'Friend activity' },
    { key: 'notifyFollowers', label: 'New followers' },
    { key: 'notifyParties', label: 'Party invites' },
    { key: 'notifyGroups', label: 'Group activity' },
    { key: 'notifyCredits', label: 'Credits on content' },
    { key: 'notifyModeration', label: 'Moderation notices' },
    { key: 'notifySystem', label: 'System announcements' },
]

export default function AccountSettingsRoute() {
    const { status, user, signIn, signOut } = useAuth()
    const { user: synced, userPending, setUser } = useSettings()
    const env = useApiEnv()

    if (status !== 'signedIn')
        return (
            <div className="flex flex-col items-start gap-3">
                <p className="text-sm text-muted">
                    Sign in to see and change the settings that follow your account.
                </p>
                <Button btnType="primary" onClick={() => void signIn()}>
                    Sign in
                </Button>
            </div>
        )

    return (
        <>
            <Section
                title="Signed in"
                hint="These settings are stored on your TMC account and apply everywhere you sign in."
            >
                <Row
                    label={user?.name ?? user?.username ?? 'Your account'}
                    hint={user?.username ? `@${user.username}` : undefined}
                    control={
                        <Button btnType="secondary" onClick={() => void signOut()}>
                            Sign out
                        </Button>
                    }
                />
                <Row
                    label="Connected devices"
                    hint="Review and remove devices from the website."
                    control={
                        <Button
                            btnType="secondary"
                            onClick={() =>
                                void openUrl(webUrl(env, '/account/security'))
                            }
                        >
                            Manage
                        </Button>
                    }
                />
            </Section>

            {userPending || !synced ? (
                <p className="text-sm text-muted">Loading your settings…</p>
            ) : (
                <Section title="Notifications">
                    {NOTIFY_ROWS.map((row) => (
                        <Row
                            key={String(row.key)}
                            label={row.label}
                            hint={row.hint}
                            control={
                                <Toggle
                                    label={row.label}
                                    checked={Boolean(synced[row.key])}
                                    onChange={(next) =>
                                        void setUser({ [row.key]: next })
                                    }
                                />
                            }
                        />
                    ))}
                </Section>
            )}
        </>
    )
}
