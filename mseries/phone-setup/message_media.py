"""Bounded public HTTPS link previews; never fetch private addresses or attachments."""
from dataclasses import dataclass
import http.client
import io
import ipaddress
import json
import re
import socket
import ssl
from urllib.parse import parse_qs, quote, urljoin, urlsplit
from PIL import Image, ImageOps

MAX_BYTES = 8 * 1024 * 1024
MAX_DECODED = 12 * 1024 * 1024
Image.MAX_IMAGE_PIXELS = 16_000_000


def media_links(text):
    found=[]
    for candidate in re.findall(r'https://[^\s<>"\x00-\x1f]+', text):
        candidate=candidate.rstrip('.,;!')
        try:
            if len(candidate)<=2048 and candidate not in found:
                path=urlsplit(candidate).path.lower()
                if youtube_id(candidate) or path.endswith(('.jpg','.jpeg','.png','.gif','.webp')):
                    found.append(candidate)
        except ValueError:continue
        if len(found)==3: break
    return found


def youtube_id(url):
    p=urlsplit(url);host=(p.hostname or '').lower()
    candidate=''
    if host in ('youtube.com','www.youtube.com','m.youtube.com'):
        if p.path=='/watch':candidate=parse_qs(p.query).get('v',[''])[0]
        elif p.path.startswith(('/shorts/','/embed/')):candidate=p.path.split('/')[2]
    elif host=='youtu.be':candidate=p.path.strip('/')
    return candidate if re.fullmatch(r'[A-Za-z0-9_-]{11}',candidate) else ''


def public_endpoint(url):
    p=urlsplit(url)
    if p.scheme!='https' or not p.hostname or p.username or p.password or p.port not in (None,443):
        raise ValueError('Only public HTTPS preview URLs are supported.')
    addresses=list(dict.fromkeys(item[4][0] for item in socket.getaddrinfo(p.hostname,443,type=socket.SOCK_STREAM)))
    if not addresses or any(not ipaddress.ip_address(a).is_global for a in addresses):
        raise ValueError('Private-network preview addresses are blocked.')
    return p,addresses[0]


class PublicHTTPS(http.client.HTTPSConnection):
    def __init__(self,host,address):
        super().__init__(host,timeout=12,context=ssl.create_default_context())
        self.address=address

    def connect(self):
        # Connect to the validated IP while retaining certificate checks/SNI for
        # the URL host. A second DNS lookup cannot redirect us into the LAN.
        raw=socket.create_connection((self.address,443),self.timeout)
        try:self.sock=self._context.wrap_socket(raw,server_hostname=self.host)
        except BaseException:raw.close();raise


def fetch(url):
    for _ in range(4):
        p,address=public_endpoint(url)
        connection=PublicHTTPS(p.hostname,address)
        try:
            connection.request('GET',p.path+('?' + p.query if p.query else '') or '/',headers={
                'User-Agent':'MSeriesMessages/1.0 (+https://github.com/techmore/ElementaryOS_MSeries)',
                'Accept-Encoding':'identity'})
            response=connection.getresponse()
            if response.status in (301,302,303,307,308):
                url=urljoin(url,response.getheader('Location',''));continue
            if response.status!=200 or response.getheader('Content-Encoding','identity').lower()!='identity':
                raise ValueError('Preview server did not return usable content.')
            length=response.getheader('Content-Length')
            if length and int(length)>MAX_BYTES:raise ValueError('Preview is too large.')
            data=response.read(MAX_BYTES+1)
            if len(data)>MAX_BYTES:raise ValueError('Preview is too large.')
            return data,response.getheader('Content-Type','').split(';')[0]
        finally:connection.close()
    raise ValueError('Too many preview redirects.')


@dataclass(frozen=True)
class Preview:
    title:str
    kind:str
    source_size:tuple
    size:tuple
    frames:tuple
    durations:tuple

    @property
    def memory(self):return sum(len(frame) for frame in self.frames)


def decode_image(data,title='Image',kind='image'):
    image=Image.open(io.BytesIO(data))
    if image.format not in ('JPEG','PNG','GIF','WEBP') or image.width*image.height>16_000_000:
        raise ValueError('Unsupported preview image.')
    count=getattr(image,'n_frames',1)
    if count>120 or image.width*image.height*count>96_000_000:
        raise ValueError('Animation is too large.')
    source_size=image.size;frames=[];durations=[];memory=0
    for index in range(count):
        image.seek(index)
        frame=ImageOps.exif_transpose(image.copy()).convert('RGBA')
        frame.thumbnail((280,220),Image.Resampling.LANCZOS)
        pixels=frame.tobytes();memory+=len(pixels)
        if memory>MAX_DECODED:raise ValueError('Decoded preview is too large.')
        frames.append(pixels);durations.append(max(40,min(int(image.info.get('duration',100) or 100),2000)))
    return Preview(title,'gif' if count>1 else kind,source_size,frame.size,tuple(frames),tuple(durations))


def load_preview(url):
    video=youtube_id(url)
    if video:
        endpoint='https://www.youtube.com/oembed?format=json&url='+quote('https://www.youtube.com/watch?v='+video,safe='')
        metadata,mime=fetch(endpoint)
        if len(metadata)>262144 or mime!='application/json':raise ValueError('Invalid YouTube metadata.')
        record=json.loads(metadata)
        thumbnail=record.get('thumbnail_url','')
        if urlsplit(thumbnail).hostname not in ('i.ytimg.com','img.youtube.com'):
            raise ValueError('Unexpected YouTube thumbnail host.')
        data,mime=fetch(thumbnail)
        return decode_image(data,str(record.get('title') or 'YouTube')[:300],'youtube')
    data,mime=fetch(url)
    if not mime.startswith('image/'):raise ValueError('This link is not an image.')
    return decode_image(data,'GIF' if mime=='image/gif' else 'Photo')
