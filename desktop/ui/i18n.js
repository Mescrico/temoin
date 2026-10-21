'use strict';
// English by default, French when the system language is French. `?lang=en|fr` forces one.
const LANG = (() => {
  const forced = new URLSearchParams(location.search).get('lang');
  if (forced === 'en' || forced === 'fr') return forced;
  return (navigator.language || 'en').toLowerCase().startsWith('fr') ? 'fr' : 'en';
})();

const plural = (n, one, many) => (n === 1 ? one : many).replace('{n}', n);

const STRINGS = {
  en: {
    appTitle: 'Témoin',
    // navigation
    navHome: 'Home', navPhones: 'Phones', navRecordings: 'Recordings', navGiveBack: 'Give back files', navSettings: 'Settings',
    // server state
    stOnline: 'Online', stConnecting: 'Connecting…', stStopped: 'Stopped', stUnreachable: 'Unreachable',
    start: 'Start', stop: 'Stop', startServer: 'Start the server', retry: 'Try again',
    heroOnline: 'Ready to receive', heroOnlineSub: 'Invited phones can film and upload from anywhere.',
    heroConnecting: 'Connecting to the Internet…', heroConnectingSub: 'This takes a few seconds.',
    heroStopped: 'The server is stopped', heroStoppedSub: 'Phones cannot upload anything until you start it.',
    heroDown: 'Not reachable from the Internet', heroDownSub: 'Check this computer’s connection, then try again.',
    invitePhone: 'Invite a phone',
    addressChanges: 'This address changes when the server restarts.',
    // home
    todoNew: 'New recordings arrived', todoNewSub: 'Recover them to watch them on this computer.', recover: 'Recover',
    todoRequests: (n) => plural(n, 'Someone asked for their files', '{n} people asked for their files'), todoRequestsSub: 'Create a one-time link for them.', giveBack: 'Give back',
    todoFirst: 'No phone yet', todoFirstSub: 'Invite the first phone to start receiving recordings.',
    statPhones: 'phones', statCaptures: 'videos and photos', statLast: 'last upload', statFree: 'free on disk', never: 'never',
    // phones
    phonesTitle: 'Phones', phonesSub: 'Invite phones and manage the ones sending to this server.',
    inviteTitle: 'Invite a phone',
    step1: 'Install the Témoin app on the phone.', step2: 'Scan this code with the camera.', step3: 'Open the link in Témoin. That’s it.',
    inviteValid: (until, max) => `Valid until ${until}` + (max ? ` · up to ${max} phones` : ''),
    inviteForever: 'No expiry',
    newCode: 'New code', manualCode: 'Code to type by hand:',
    createCode: 'Create an invitation code', validFor: 'Valid for', h6: '6 h', h24: '24 h', h48: '2 days', d7: '7 days', unlimited: 'No limit',
    startToInvite: 'Start the server to invite phones.',
    tunnelHint: 'If the server restarts, its address changes: phones then scan a new code and keep their identity.',
    registered: (n) => (n ? plural(n, '{n} registered phone', '{n} registered phones') : 'Registered phones'),
    noPhones: 'No phone has joined yet.',
    lastUpload: (when) => `Last upload ${when}`, noUpload: 'Nothing sent yet',
    blocked: 'Blocked', block: 'Block', unblock: 'Unblock', rename: 'Rename',
    blockConfirmTitle: 'Block this phone?', blockConfirm: (n) => `${n} will not be able to send anything. Its recordings stay on the server.`,
    renameTitle: 'Rename this phone', renameHint: 'Avoid real names: the server could be seized.',
    // recordings
    recTitle: 'Recordings', recSub: 'Recordings arrive encrypted. Decrypt them on this computer to watch them.',
    recNew: 'New recordings to recover', recNewSub: 'They will be decrypted into the folder below.',
    recUpToDate: 'Everything is recovered', recUpToDateSub: 'New recordings will show up here.',
    saveTo: 'Save to', change: 'Change', recoverAgain: 'Decrypt everything again',
    recDone: (n) => plural(n, '{n} video or photo decrypted', '{n} videos and photos decrypted'), openFolder: 'Open the folder',
    recNothing: 'Nothing new since last time.', recUnreadable: (n) => plural(n, '{n} file could not be read.', '{n} files could not be read.'),
    neverDeleted: 'Nothing is ever deleted from the server.',
    decrypting: 'Decrypting…', showFiles: 'Show files', hideFiles: 'Hide files',
    // give back
    gbTitle: 'Give back files', gbSub: 'Give someone what they filmed, through a link that opens only once.',
    gbStep1: 'Code', gbStep2: 'Check', gbStep3: 'Link',
    gbAsk: 'Ask the person for the code shown in their app', gbAskSub: 'Settings → Get my files back',
    continue: 'Continue', back: 'Back',
    gbFound: (n, size) => `${plural(n, '{n} recording', '{n} recordings')} · ${size}`,
    gbLinkValid: 'The link stays valid for', d1: '1 day', d3: '3 days', d30: '30 days',
    createLink: 'Create the link', preparing: 'Preparing the link…', preparingSub: 'Decrypting, re-encrypting and uploading. This can take a minute.',
    gbReady: 'The link is ready', gbReadyTip1: 'Send it another way than the code: Signal, or show the QR code in person.', gbReadyTip2: 'It opens once. The person gets a .zip with their videos and photos.',
    copy: 'Copy', copied: 'Link copied', another: 'Give back other files',
    gbCancelled: 'This request was cancelled.', gbCheckHint: 'Check that the person sees this same code on their phone.',
    linksTitle: 'Requests and links', answer: 'Answer', noLinks: 'No link created yet.',
    lkWaiting: 'Waiting for a link', lkUploading: 'Preparing', lkReady: 'Not opened yet', lkOpened: 'Being downloaded', lkDone: 'Retrieved', lkExpired: 'Expired', lkRevoked: 'Revoked',
    revoke: 'Revoke', revokeTitle: 'Revoke this link?', revokeText: 'It will stop working. Nothing is deleted.',
    expiresOn: (d) => `expires ${d}`,
    // settings
    setTitle: 'Settings', setGeneral: 'General', setName: 'Server name', setNameSub: 'Shown in the app when a phone joins.', save: 'Save', saved: 'Saved',
    setAutostart: 'Start automatically', setAutostartSub: 'Témoin opens in the background when you log in to this computer and starts the server. Recommended: nothing is missed after a restart.',
    setAccess: 'Access from the Internet',
    accAuto: 'Automatic (recommended)', accAutoSub: 'Works everywhere, nothing to set up. The address changes when the server restarts. Cloudflare relays the traffic and sees who connects, never the content.',
    accDomain: 'My own domain name', accDomainSub: 'Fixed address, no middleman. Needs a domain pointing to your router and port 443 forwarded to this computer.',
    domain: 'Domain name', domainPh: 'myserver.duckdns.org', email: 'E-mail for Let’s Encrypt (optional)',
    restartNote: 'The server restarts to apply this.',
    setStorage: 'Storage', setData: 'Recordings received', setRecovered: 'Decrypted recordings', open: 'Open', freeSpace: (s) => `${s} free`,
    setSecurity: 'Security', setKey: 'Server key', setKeySub: 'Keep a copy elsewhere (USB stick). Without it, recordings cannot be read if this computer breaks.',
    saveKeyCopy: 'Save a copy', keySaved: 'Copy saved',
    setAdvanced: 'Advanced', setAdmin: 'Full administration', setAdminSub: 'Detailed journal, invitations and requests, in a separate window.', openAdmin: 'Open',
    setAbout: 'About', aboutText: 'Free software (AGPL-3.0). Not audited yet.', sourceCode: 'Source code',
    // passphrase dialog
    unlockTitle: 'Enter your passphrase', unlockSub: 'It unlocks the key, only for this operation.', passphrase: 'Passphrase', cancel: 'Cancel', ok: 'Continue',
    show: 'Show', hide: 'Hide',
    // onboarding
    obWelcome: 'Welcome to Témoin', obWelcomeSub: 'This computer will receive the videos your group films, encrypted on the phones. Only your passphrase can open them.',
    obF1: 'Encrypted on the phone', obF1Sub: 'Nothing readable stays on the phone or on the way.',
    obF2: 'Reachable from anywhere', obF2Sub: 'No router setup, no account.',
    obF3: 'Give everyone their files back', obF3Sub: 'One-time links, no app needed.',
    getStarted: 'Get started',
    obName: 'Name your server', obNameSub: 'Phones show this name when they join it.', obNamePh: 'Neighbourhood collective',
    obFolder: 'Where should recordings go?', obFolderSub: 'They stay encrypted here. Choose a disk with room: about 1 GB per hour of video.',
    obPass: 'Choose a passphrase', obPassSub: 'It protects the key that opens the videos. A few random words are easy to remember and hard to guess.',
    obPassExample: 'Example: frog piano tide monday', obPassAgain: 'Type it again',
    strength: ['Too short', 'Weak', 'Good', 'Strong'],
    obPassNoted: 'I wrote my passphrase down somewhere safe. Without it, nobody can ever read the recordings.',
    createKey: 'Create my key', creatingKey: 'Creating the key…',
    obBackup: 'Save a backup of the key', obBackupSub: 'Put a copy on a USB stick kept elsewhere. If this computer breaks or disappears, the copy and your passphrase let you read the recordings again.',
    later: 'Later', backupLaterTitle: 'Continue without a backup?', backupLaterText: 'You can save it later in Settings → Security.',
    obReady: 'All set', obReadySub: 'The server is starting. Next: invite a first phone.', openApp: 'Open Témoin',
    passMismatch: 'The two passphrases differ.',
    // time
    ago: (ms) => relative(ms),
    errors: {
      no_passphrase: 'Enter your passphrase.',
      passphrase_short: 'The passphrase must be at least 10 characters long.',
      key_exists: 'A key already exists for this server.',
      stop_first: 'Stop the server first.',
      not_configured: 'The server is not set up yet.',
      domain_required: 'Enter the domain name that points to your router.',
      wrong_passphrase: 'Wrong passphrase.',
      unknown_code: 'This code does not exist. Check it with the person.',
    },
  },
  fr: {
    appTitle: 'Témoin',
    navHome: 'Accueil', navPhones: 'Téléphones', navRecordings: 'Enregistrements', navGiveBack: 'Rendre des fichiers', navSettings: 'Réglages',
    stOnline: 'En ligne', stConnecting: 'Connexion…', stStopped: 'Arrêté', stUnreachable: 'Injoignable',
    start: 'Démarrer', stop: 'Arrêter', startServer: 'Démarrer le serveur', retry: 'Réessayer',
    heroOnline: 'Prêt à recevoir', heroOnlineSub: 'Les téléphones invités peuvent filmer et envoyer depuis n’importe où.',
    heroConnecting: 'Connexion à Internet…', heroConnectingSub: 'Quelques secondes.',
    heroStopped: 'Le serveur est arrêté', heroStoppedSub: 'Les téléphones ne peuvent rien envoyer tant qu’il n’est pas démarré.',
    heroDown: 'Injoignable depuis Internet', heroDownSub: 'Vérifiez la connexion de cet ordinateur, puis réessayez.',
    invitePhone: 'Inviter un téléphone',
    addressChanges: 'Cette adresse change quand le serveur redémarre.',
    todoNew: 'De nouveaux enregistrements sont arrivés', todoNewSub: 'Récupérez-les pour les regarder sur cet ordinateur.', recover: 'Récupérer',
    todoRequests: (n) => plural(n, 'Quelqu’un demande ses fichiers', '{n} personnes demandent leurs fichiers'), todoRequestsSub: 'Créez-lui un lien à usage unique.', giveBack: 'Rendre',
    todoFirst: 'Aucun téléphone pour l’instant', todoFirstSub: 'Invitez le premier téléphone pour commencer à recevoir.',
    statPhones: 'téléphones', statCaptures: 'vidéos et photos', statLast: 'dernier envoi', statFree: 'libres sur le disque', never: 'jamais',
    phonesTitle: 'Téléphones', phonesSub: 'Invitez des téléphones et gérez ceux qui envoient vers ce serveur.',
    inviteTitle: 'Inviter un téléphone',
    step1: 'Installez l’app Témoin sur le téléphone.', step2: 'Scannez ce code avec l’appareil photo.', step3: 'Ouvrez le lien dans Témoin. C’est prêt.',
    inviteValid: (until, max) => `Valable jusqu’au ${until}` + (max ? ` · jusqu’à ${max} téléphones` : ''),
    inviteForever: 'Sans date limite',
    newCode: 'Nouveau code', manualCode: 'Code à saisir à la main :',
    createCode: 'Créer un code d’invitation', validFor: 'Valable', h6: '6 h', h24: '24 h', h48: '2 jours', d7: '7 jours', unlimited: 'Sans limite',
    startToInvite: 'Démarrez le serveur pour inviter des téléphones.',
    tunnelHint: 'Si le serveur redémarre, son adresse change : les téléphones rescannent alors un code et gardent leur identité.',
    registered: (n) => (n ? plural(n, '{n} téléphone inscrit', '{n} téléphones inscrits') : 'Téléphones inscrits'),
    noPhones: 'Aucun téléphone n’a encore rejoint le serveur.',
    lastUpload: (when) => `Dernier envoi ${when}`, noUpload: 'Rien envoyé pour l’instant',
    blocked: 'Bloqué', block: 'Bloquer', unblock: 'Débloquer', rename: 'Renommer',
    blockConfirmTitle: 'Bloquer ce téléphone ?', blockConfirm: (n) => `${n} ne pourra plus rien envoyer. Ses enregistrements restent sur le serveur.`,
    renameTitle: 'Renommer ce téléphone', renameHint: 'Évitez les vrais noms : le serveur pourrait être saisi.',
    recTitle: 'Enregistrements', recSub: 'Les enregistrements arrivent chiffrés. Déchiffrez-les sur cet ordinateur pour les regarder.',
    recNew: 'Des enregistrements sont à récupérer', recNewSub: 'Ils seront déchiffrés dans le dossier ci-dessous.',
    recUpToDate: 'Tout est récupéré', recUpToDateSub: 'Les nouveaux enregistrements apparaîtront ici.',
    saveTo: 'Enregistrer dans', change: 'Changer', recoverAgain: 'Tout redéchiffrer',
    recDone: (n) => plural(n, '{n} vidéo ou photo déchiffrée', '{n} vidéos et photos déchiffrées'), openFolder: 'Ouvrir le dossier',
    recNothing: 'Rien de nouveau depuis la dernière fois.', recUnreadable: (n) => plural(n, '{n} fichier illisible.', '{n} fichiers illisibles.'),
    neverDeleted: 'Rien n’est jamais supprimé du serveur.',
    decrypting: 'Déchiffrement…', showFiles: 'Voir les fichiers', hideFiles: 'Masquer les fichiers',
    gbTitle: 'Rendre des fichiers', gbSub: 'Rendez à une personne ce qu’elle a filmé, par un lien qui ne s’ouvre qu’une fois.',
    gbStep1: 'Code', gbStep2: 'Vérification', gbStep3: 'Lien',
    gbAsk: 'Demandez à la personne le code affiché dans son app', gbAskSub: 'Réglages → Récupérer mes fichiers',
    continue: 'Continuer', back: 'Retour',
    gbFound: (n, size) => `${plural(n, '{n} enregistrement', '{n} enregistrements')} · ${size}`,
    gbLinkValid: 'Le lien reste valable', d1: '1 jour', d3: '3 jours', d30: '30 jours',
    createLink: 'Créer le lien', preparing: 'Préparation du lien…', preparingSub: 'Déchiffrement, rechiffrement et envoi. Cela peut prendre une minute.',
    gbReady: 'Le lien est prêt', gbReadyTip1: 'Envoyez-le par un autre moyen que le code : Signal, ou montrez le QR en vrai.', gbReadyTip2: 'Il ne s’ouvre qu’une fois. La personne récupère un .zip avec ses vidéos et photos.',
    copy: 'Copier', copied: 'Lien copié', another: 'Rendre d’autres fichiers',
    gbCancelled: 'Cette demande a été annulée.', gbCheckHint: 'Vérifiez que la personne voit ce même code sur son téléphone.',
    linksTitle: 'Demandes et liens', answer: 'Répondre', noLinks: 'Aucun lien créé pour l’instant.',
    lkWaiting: 'En attente d’un lien', lkUploading: 'En préparation', lkReady: 'Pas encore ouvert', lkOpened: 'En cours de téléchargement', lkDone: 'Récupéré', lkExpired: 'Expiré', lkRevoked: 'Révoqué',
    revoke: 'Révoquer', revokeTitle: 'Révoquer ce lien ?', revokeText: 'Il ne fonctionnera plus. Rien n’est supprimé.',
    expiresOn: (d) => `expire le ${d}`,
    setTitle: 'Réglages', setGeneral: 'Général', setName: 'Nom du serveur', setNameSub: 'Affiché dans l’app quand un téléphone le rejoint.', save: 'Enregistrer', saved: 'Enregistré',
    setAutostart: 'Démarrer automatiquement', setAutostartSub: 'Témoin s’ouvre en arrière-plan à l’ouverture de votre session et démarre le serveur. Recommandé : rien n’est manqué après un redémarrage.',
    setAccess: 'Accès depuis Internet',
    accAuto: 'Automatique (recommandé)', accAutoSub: 'Fonctionne partout, rien à configurer. L’adresse change quand le serveur redémarre. Cloudflare relaie le trafic et voit qui se connecte, jamais le contenu.',
    accDomain: 'Mon propre nom de domaine', accDomainSub: 'Adresse fixe, sans intermédiaire. Il faut un nom de domaine qui pointe vers votre box et le port 443 redirigé vers cet ordinateur.',
    domain: 'Nom de domaine', domainPh: 'monserveur.duckdns.org', email: 'E-mail pour Let’s Encrypt (facultatif)',
    restartNote: 'Le serveur redémarre pour appliquer le changement.',
    setStorage: 'Stockage', setData: 'Enregistrements reçus', setRecovered: 'Enregistrements déchiffrés', open: 'Ouvrir', freeSpace: (s) => `${s} libres`,
    setSecurity: 'Sécurité', setKey: 'Clé du serveur', setKeySub: 'Gardez-en une copie ailleurs (clé USB). Sans elle, les enregistrements sont illisibles si cet ordinateur tombe en panne.',
    saveKeyCopy: 'Enregistrer une copie', keySaved: 'Copie enregistrée',
    setAdvanced: 'Avancé', setAdmin: 'Administration complète', setAdminSub: 'Journal détaillé, invitations et demandes, dans une fenêtre à part.', openAdmin: 'Ouvrir',
    setAbout: 'À propos', aboutText: 'Logiciel libre (AGPL-3.0). Pas encore audité.', sourceCode: 'Code source',
    unlockTitle: 'Saisissez votre phrase de passe', unlockSub: 'Elle déverrouille la clé, seulement pour cette opération.', passphrase: 'Phrase de passe', cancel: 'Annuler', ok: 'Continuer',
    show: 'Afficher', hide: 'Masquer',
    obWelcome: 'Bienvenue dans Témoin', obWelcomeSub: 'Cet ordinateur va recevoir les vidéos filmées par votre groupe, chiffrées sur les téléphones. Seule votre phrase de passe peut les ouvrir.',
    obF1: 'Chiffré sur le téléphone', obF1Sub: 'Rien de lisible ne reste sur le téléphone ni en chemin.',
    obF2: 'Joignable de partout', obF2Sub: 'Sans réglage de la box, sans compte.',
    obF3: 'Chacun récupère ses fichiers', obF3Sub: 'Liens à usage unique, sans installer l’app.',
    getStarted: 'Commencer',
    obName: 'Donnez un nom à votre serveur', obNameSub: 'Les téléphones affichent ce nom quand ils le rejoignent.', obNamePh: 'Collectif du quartier',
    obFolder: 'Où ranger les enregistrements ?', obFolderSub: 'Ils restent chiffrés ici. Choisissez un disque avec de la place : environ 1 Go par heure de vidéo.',
    obPass: 'Choisissez une phrase de passe', obPassSub: 'Elle protège la clé qui ouvre les vidéos. Quelques mots au hasard sont faciles à retenir et difficiles à deviner.',
    obPassExample: 'Exemple : grenouille piano marée lundi', obPassAgain: 'Retapez-la',
    strength: ['Trop courte', 'Faible', 'Bonne', 'Solide'],
    obPassNoted: 'J’ai noté ma phrase de passe en lieu sûr. Sans elle, personne ne pourra jamais lire les enregistrements.',
    createKey: 'Créer ma clé', creatingKey: 'Création de la clé…',
    obBackup: 'Faites une copie de secours de la clé', obBackupSub: 'Mettez une copie sur une clé USB rangée ailleurs. Si cet ordinateur tombe en panne ou disparaît, cette copie et votre phrase de passe permettent de relire les enregistrements.',
    later: 'Plus tard', backupLaterTitle: 'Continuer sans copie de secours ?', backupLaterText: 'Vous pourrez la faire plus tard dans Réglages → Sécurité.',
    obReady: 'Tout est prêt', obReadySub: 'Le serveur démarre. Prochaine étape : inviter un premier téléphone.', openApp: 'Ouvrir Témoin',
    passMismatch: 'Les deux phrases de passe sont différentes.',
    ago: (ms) => relative(ms),
    errors: {
      no_passphrase: 'Saisissez votre phrase de passe.',
      passphrase_short: 'La phrase de passe doit faire au moins 10 caractères.',
      key_exists: 'Une clé existe déjà pour ce serveur.',
      stop_first: 'Arrêtez d’abord le serveur.',
      not_configured: 'Le serveur n’est pas encore configuré.',
      domain_required: 'Indiquez le nom de domaine qui pointe vers votre box.',
      wrong_passphrase: 'Phrase de passe incorrecte.',
      unknown_code: 'Ce code n’existe pas. Vérifiez-le avec la personne.',
    },
  },
};

function t(key, ...args) {
  const v = STRINGS[LANG][key] ?? STRINGS.en[key] ?? key;
  return typeof v === 'function' ? v(...args) : v;
}

/** Errors from Rust come as "@code" (translated here) or as plain technical text. */
function errText(e) {
  const s = String(e);
  if (s.startsWith('@')) return STRINGS[LANG].errors[s.slice(1)] || STRINGS.en.errors[s.slice(1)] || s;
  return s;
}

const rtfs = { long: new Intl.RelativeTimeFormat(LANG, { numeric: 'auto' }), short: new Intl.RelativeTimeFormat(LANG, { numeric: 'auto', style: 'short' }) };
/** "5 minutes ago", "il y a 5 minutes"… (`short`: "5 min. ago", "il y a 5 min") */
function relative(iso, style = 'long') {
  if (!iso) return t('never');
  const rtf = rtfs[style];
  const s = Math.round((new Date(iso) - Date.now()) / 1000);
  const a = Math.abs(s);
  if (a < 45) return rtf.format(0, 'second');
  if (a < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (a < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  return rtf.format(Math.round(s / 86400), 'day');
}
function fmtDate(iso) {
  return iso ? new Date(iso).toLocaleString(LANG === 'fr' ? 'fr-FR' : undefined, { day: 'numeric', month: 'long', hour: '2-digit', minute: '2-digit' }) : '';
}
function fmtBytes(n) {
  if (n == null) return '–';
  const u = LANG === 'fr' ? ['o', 'Ko', 'Mo', 'Go', 'To'] : ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0; while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return (i ? n.toFixed(n < 10 ? 1 : 0) : n) + ' ' + u[i];
}
