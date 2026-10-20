import io
import unittest
from unittest.mock import patch
from PIL import Image
from message_media import decode_image, media_links, public_endpoint, youtube_id


class MediaTests(unittest.TestCase):
    def test_only_expected_https_media_links_are_candidates(self):
        text='https://www.youtube.com/watch?v=aqz-KE-bpKQ https://example.org/a.gif https://[invalid.jpg http://example.org/b.jpg <img src=x>'
        self.assertEqual(media_links(text),['https://www.youtube.com/watch?v=aqz-KE-bpKQ','https://example.org/a.gif'])
        self.assertEqual(youtube_id('https://youtube.com.evil.example/watch?v=aqz-KE-bpKQ'),'')

    def test_private_dns_credentials_and_nonstandard_ports_are_blocked(self):
        with patch('message_media.socket.getaddrinfo',return_value=[(2,1,6,'',('127.0.0.1',443))]):
            with self.assertRaises(ValueError):public_endpoint('https://example.org/a.jpg')
        for url in ['http://example.org/a.jpg','https://user:secret@example.org/a.jpg','https://example.org:8443/a.jpg']:
            with self.assertRaises(ValueError):public_endpoint(url)

    def test_photo_is_scaled_without_losing_source_resolution(self):
        out=io.BytesIO();Image.new('RGB',(1000,500),'blue').save(out,format='JPEG')
        preview=decode_image(out.getvalue())
        self.assertEqual(preview.source_size,(1000,500))
        self.assertEqual(preview.size,(280,140))
        self.assertEqual(len(preview.frames),1)

    def test_gif_keeps_animation_frames_and_timing(self):
        images=[Image.new('RGB',(40,40),c) for c in ('red','blue')]
        out=io.BytesIO();images[0].save(out,format='GIF',save_all=True,append_images=images[1:],duration=[80,120],loop=0)
        preview=decode_image(out.getvalue())
        self.assertEqual(preview.kind,'gif')
        self.assertEqual(preview.durations,(80,120))
        self.assertNotEqual(preview.frames[0],preview.frames[1])

    def test_arbitrary_payload_is_not_an_image(self):
        with self.assertRaises(Exception):decode_image(b'<html>not an image</html>')


if __name__=='__main__':unittest.main()
