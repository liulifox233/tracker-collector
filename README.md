# tracker-collector

![metadata](https://github.com/liulifox233/tracker-collector/actions/workflows/deploy.yml/badge.svg)

## How it works?

1. Fetch the tracker list from urls in trackers.yml.
2. Merge the tracker list with the trackers from the request.
3. (scheduled) Connect to aria2 with JSON-RPC, then add the trackers to aria2.

## Usage

1. Set urls in trackers.yml.

2. Set CLOUDFLARE_API_TOKEN in repository secrets.

3. Deploy to Cloudflare Workers

   [![Deploy to Cloudflare Workers](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/liulifox233/tacker-collector)

4. Set ARIA2_URL, SECRET_KEY and schedule in Cloudflare Workers settings.

5. Done.
