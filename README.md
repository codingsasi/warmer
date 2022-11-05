# warmer
A CDN cache warmer in rust for the sitemap.xml files that look like this:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<?xml-stylesheet type="text/xsl" href="/sitemap.xsl"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9" xmlns:xhtml="http://www.w3.org/1999/xhtml">
<url>
    <loc>https://abh.ai/</loc>
    <lastmod>2022-06-25T20:46Z</lastmod>
    <changefreq>daily</changefreq>
    <priority>1.0</priority>
</url>
<url>
    <loc>https://abh.ai/photos/nature</loc>
    <lastmod>2022-09-25T05:33Z</lastmod>
    <changefreq>monthly</changefreq>
    <priority>0.7</priority>
</url>
<url>
    <loc>https://abh.ai/portraits</loc>
    <lastmod>2022-09-24T18:42Z</lastmod>
    <changefreq>monthly</changefreq>
    <priority>0.7</priority>
</url>
</urlset>
```

## Other examples of sitemaps that work

- https://abh.ai/sitemap.xml
- https://qed42.com/sitemap.xml
- https://www.australia.gov.au/sitemap.xml
- https://www.alkhaleej.ae/sitemap.xml?page=1
- https://www.axelerant.com/sitemap.xml
- https://ffw.com/sitemap.xml

## Usage

Download (from [here](https://github.com/codingsasi/warmer/releases)) and run the executable binary on linux with the following command
```
warmer http(s)://someurl.com/sitemap.xml interl
 - ./warmer https://abh.ai/sitemap.xml 5
 - ./warmer https://abh.ai/sitemap.xml 1
```
- The interval value should be specified in seconds, and it's how long the warmer waiting before loading the next URL in the sitemap.
- The default interval is 5 seconds. No interval (0s) is allowed but use only if you want to DDOS your own site.
- The complete url should be specified with scheme as well.

### Notes
Large sitemaps, that include other zipped or gzipped sitemaps are not supported yet. I'll release that as and when I get time. But for most sitemaps this should warm it just fine.
Currently on supported on 64-bit linux OS.