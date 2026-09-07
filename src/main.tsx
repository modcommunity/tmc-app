import React from 'react'
import ReactDOM from 'react-dom/client'

import App from './App'
import './styles/app.css'

/*
 * Context-menu and refresh shortcuts are browser affordances, and in a shipped
 * app they read as bugs — a right-click offering "View page source" over a mod
 * listing, or F5 wiping the user's scroll position and in-flight install. Both
 * stay on in dev, where they are how the app is debugged.
 */
if (!import.meta.env.DEV) {
    document.addEventListener('contextmenu', (e) => e.preventDefault())

    document.addEventListener('keydown', (e) => {
        const blocked =
            e.key === 'F5' ||
            (e.key === 'F7' && !e.ctrlKey) ||
            ((e.ctrlKey || e.metaKey) &&
                ['r', 'p', 'f', 'g', 'u'].includes(e.key.toLowerCase()))

        if (blocked) e.preventDefault()
    })
}

const root = document.getElementById('root')

if (!root) throw new Error('#root is missing from index.html')

ReactDOM.createRoot(root).render(
    <React.StrictMode>
        <App />
    </React.StrictMode>
)
