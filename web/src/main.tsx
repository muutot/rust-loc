import React from 'react'
import ReactDOM from 'react-dom/client'
import { App } from './App'
import { captureToken } from './lib/api'
import './styles.css'

captureToken()
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
