"""
AWS Lambda Proxy for Polymarket CLOB API
Bypasses Cloudflare by using Lambda's rotating IP pool

Deploy to AWS Lambda (Python 3.11 runtime)
Set environment variables: (none needed - bot sends full request)
"""

import json
import urllib.request
import urllib.error


def lambda_handler(event, context):
    """
    Proxy POST requests to Polymarket CLOB API

    Expected event format:
    {
        "path": "/order",
        "method": "POST",
        "headers": {
            "POLY_ADDRESS": "...",
            "POLY_API_KEY": "...",
            "POLY_PASSPHRASE": "...",
            "POLY_TIMESTAMP": "...",
            "POLY_SIGNATURE": "...",
            "Content-Type": "application/json"
        },
        "body": "{...order JSON...}"
    }
    """

    try:
        # Parse input
        path = event.get('path', '/order')
        method = event.get('method', 'POST')
        headers = event.get('headers', {})
        body = event.get('body', '')

        # Build request to Polymarket
        url = f"https://clob.polymarket.com{path}"

        # Create request
        req = urllib.request.Request(
            url,
            data=body.encode('utf-8') if body else None,
            headers=headers,
            method=method
        )

        # Add browser-like headers
        req.add_header('User-Agent', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36')
        req.add_header('Accept', 'application/json, text/plain, */*')
        req.add_header('Accept-Language', 'en-US,en;q=0.9')

        # Make request
        with urllib.request.urlopen(req, timeout=30) as response:
            response_body = response.read().decode('utf-8')
            return {
                'statusCode': response.status,
                'headers': dict(response.headers),
                'body': response_body
            }

    except urllib.error.HTTPError as e:
        return {
            'statusCode': e.code,
            'body': json.dumps({
                'error': str(e),
                'reason': e.reason,
                'response': e.read().decode('utf-8') if e.fp else ''
            })
        }
    except Exception as e:
        return {
            'statusCode': 500,
            'body': json.dumps({'error': str(e)})
        }


# For local testing
if __name__ == '__main__':
    # Test event
    test_event = {
        'path': '/markets',
        'method': 'GET',
        'headers': {},
        'body': ''
    }
    result = lambda_handler(test_event, None)
    print(json.dumps(result, indent=2))
