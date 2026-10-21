# Privacy policy

*Applies to the Témoin apps for Android and iOS and to the Témoin server software. Last updated: 2026-10-21. [Version française plus bas.](#politique-de-confidentialité)*

## Short version

The Témoin project collects **nothing**. There is no account, no analytics, no advertising, no crash reporting and no tracker in the apps. Recordings go only to the server **you** choose by scanning an invitation, encrypted so that this server cannot read them.

## What the app sends, and to whom

The app talks only to the server whose invitation you scanned. That server is run by an organizer (a person, a group, an organization), not by the Témoin project. The app sends it:

- **Your recordings** (photos, video, sound), encrypted on the phone before they leave it, with the organizer's public key. The server stores them but cannot decrypt them. Only the organizer's private key can.
- **Inside the encrypted files**: the time of the recording and, only if you turn it on in the settings, your position. Nobody can read these without the organizer's key.
- **In clear, as needed to receive the files**: an anonymous key created on your phone for that server, the size of each file, the time it arrived. The server knows a pseudonym ("Phone #12"), never your name or phone number.
- **Network data**: like any Internet service, the server (and, if it uses one, its Cloudflare tunnel) sees the IP address the request comes from.

The app does not send anything to the Témoin project or to any third party.

## Permissions

- **Camera and microphone**: to film. What is recorded is encrypted immediately.
- **Location** (optional, off by default): to add your position inside the encrypted file.
- **Notifications**: to show that a recording or an upload is in progress.

## What stays on the phone

Nothing readable. Recordings exist on the phone only encrypted, while waiting to be sent; once the server confirms them they are removed from the phone. The app keeps its settings and the anonymous key for each server you joined.

## Keeping and deleting data

Servers are run by organizers, who decide what happens to the recordings. The server is designed as **write-only**: it never deletes what it received, so that evidence cannot be erased by force. To get your own recordings back, use *Settings → Get my files back*; for anything else, contact the organizer of the server you joined.

## Children

Témoin is not directed at children.

## Contact

Questions: open an issue at https://github.com/temoin-app/temoin. Security problems: use private reporting (*Security → Report a vulnerability*) on that page.

---

# Politique de confidentialité

*S'applique aux apps Témoin pour Android et iOS et au logiciel serveur Témoin. Dernière mise à jour : 21/10/2026.*

## En bref

Le projet Témoin ne collecte **rien**. Pas de compte, pas de statistiques, pas de publicité, pas de rapport de plantage, aucun traceur dans les apps. Les enregistrements vont uniquement au serveur que **vous** choisissez en scannant une invitation, chiffrés de façon que ce serveur ne puisse pas les lire.

## Ce que l'app envoie, et à qui

L'app ne parle qu'au serveur dont vous avez scanné l'invitation. Ce serveur est tenu par un organisateur (une personne, un groupe, une organisation), pas par le projet Témoin. L'app lui envoie :

- **Vos enregistrements** (photos, vidéo, son), chiffrés sur le téléphone avant de le quitter, avec la clé publique de l'organisateur. Le serveur les garde mais ne peut pas les déchiffrer. Seule la clé privée de l'organisateur le peut.
- **Dans les fichiers chiffrés** : l'heure de l'enregistrement et, seulement si vous l'activez dans les réglages, votre position. Personne ne peut les lire sans la clé de l'organisateur.
- **En clair, pour recevoir les fichiers** : une clé anonyme créée sur votre téléphone pour ce serveur, la taille de chaque fichier, l'heure d'arrivée. Le serveur connaît un pseudonyme (« Phone #12 »), jamais votre nom ni votre numéro.
- **Données réseau** : comme tout service Internet, le serveur (et, s'il en utilise un, son tunnel Cloudflare) voit l'adresse IP d'où vient la requête.

L'app n'envoie rien au projet Témoin ni à aucun tiers.

## Autorisations

- **Appareil photo et micro** : pour filmer. Ce qui est enregistré est chiffré aussitôt.
- **Position** (facultative, désactivée par défaut) : pour ajouter votre position dans le fichier chiffré.
- **Notifications** : pour indiquer qu'un enregistrement ou un envoi est en cours.

## Ce qui reste sur le téléphone

Rien de lisible. Les enregistrements n'existent sur le téléphone que chiffrés, en attente d'envoi ; une fois confirmés par le serveur, ils sont retirés du téléphone. L'app garde ses réglages et la clé anonyme de chaque serveur rejoint.

## Conservation et suppression

Les serveurs sont tenus par des organisateurs, qui décident du sort des enregistrements. Le serveur est conçu **en écriture seule** : il ne supprime jamais ce qu'il a reçu, pour que des preuves ne puissent pas être effacées sous la contrainte. Pour récupérer vos propres enregistrements, utilisez *Réglages → Récupérer mes fichiers* ; pour le reste, contactez l'organisateur du serveur rejoint.

## Enfants

Témoin ne s'adresse pas aux enfants.

## Contact

Questions : ouvrez une issue sur https://github.com/temoin-app/temoin. Problèmes de sécurité : utilisez le signalement privé (*Security → Report a vulnerability*) sur cette page.
