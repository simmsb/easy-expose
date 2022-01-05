# Easy expose

A really simple way to expose some service behind a NAT, similar to
[rathole](https://github.com/rapiz1/rathole) and
[frp](https://github.com/fatedier/frp).

WARNING: This does not secure the channel, or even do NAT busting. You should
already have an overlay network that creates the secure channel, like tailscale,
or even just a plain wireguard vpn.

The main benefit of this over rathole and such is that the latency should be the
lowest possible, as the redirect is setup using nftables, rather than two
userspace programs.

## Usage

```
easy-expose 0.1.0
Set up a packet redirect on some remote host that forwards packets to you

example: `easy-expose test_redir tcp root@vps 9912 100.82.95.116:9912`

USAGE:
    easy-expose [OPTIONS] <IDENTIFIER> <MODE> <DESTINATION> <REMOTE> <LOCAL>

ARGS:
    <IDENTIFIER>
            A unique name to identify this forwarding instance

    <MODE>
            What type of packet to forward

            [possible values: udp, tcp]

    <DESTINATION>
            The remote host to expose on [format: a ssh destination]

            nftables needs to be installed on this host, the user to connect as also needs
            permission to run `nft` (aka: root)

    <REMOTE>
            The remote port to expose on

    <LOCAL>
            Where to forward packets to [format: <ip/hostname>:<port>]

OPTIONS:
    -h, --help
            Print help information

    -i, --identity <FILE>
            The ssh identity file to use

    -V, --version
            Print version information
```


## Docker usage

You can run this through the docker image: `ghcr.io/simmsb/easy-expose:latest`, for example:

```sh
$ docker run -v ~/.ssh/id_rsa:/id_rsa \
    ghcr.io/simmsb/easy-expose:latest \
        test_redir tcp root@<ip_addr> 9912 100.82.95.116:9912 -i /id_rsa
```
