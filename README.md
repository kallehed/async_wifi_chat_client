# Async wifi local chat client

Uses UDP to broadcast your existance,
also listens at the same time for UDP broadcasts from other people
then you can connect to other people with TCP, and write "quit" to disconnect

To actually make this run you have to open up your firewall to allow UDP and TCP traffic over port 2000.

Exercise in using async rust with Tokio. Less jank than using just threads, but also more jank in some other ways (probably because I used the wrong patterns for some things, like distributing STDIN to different tasks)

Also your wifi has to NOT filter UDP broadcasts, which they probably do if it's professional.
