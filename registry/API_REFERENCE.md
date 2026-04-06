# Verum Registry - API Reference

## Base URL

```
http://localhost:8080
```

## Endpoints

### Health Check

#### GET /health

Check if the registry is running and healthy.

**Response:**
```json
{
  "status": "healthy",
  "version": "1.0.0"
}
```

**Status Codes:**
- `200 OK`: Registry is healthy

---

### Packages

#### GET /api/packages

List all packages in the registry.

**Response:**
```json
{
  "packages": [
    {
      "id": "std",
      "version": "0.1.0"
    },
    {
      "id": "http",
      "version": "0.1.0"
    }
  ],
  "total": 2
}
```

**Status Codes:**
- `200 OK`: Successfully retrieved package list

---

#### GET /api/packages/{name}

Get detailed information about a specific package.

**Parameters:**
- `name` (path): Package name

**Response:**
```json
{
  "id": "std",
  "description": "Standard library with core types and functions",
  "license": "MIT",
  "version": "0.1.0",
  "downloads": 0
}
```

**Status Codes:**
- `200 OK`: Package found
- `404 Not Found`: Package does not exist

**Error Response:**
```json
{
  "error": "Package not found: nonexistent"
}
```

---

#### POST /api/packages

Create a new package.

**Request Body:**
```json
{
  "id": "my-package",
  "description": "My awesome package",
  "license": "MIT"
}
```

**Response:**
```json
{
  "id": "my-package",
  "description": "My awesome package",
  "license": "MIT",
  "version": "0.1.0",
  "downloads": 0
}
```

**Status Codes:**
- `201 Created`: Package created successfully
- `409 Conflict`: Package already exists
- `400 Bad Request`: Invalid request body
- `401 Unauthorized`: Authentication required

---

#### PUT /api/packages/{name}

Update an existing package.

**Parameters:**
- `name` (path): Package name

**Request Body:**
```json
{
  "description": "Updated description",
  "license": "Apache-2.0"
}
```

**Status Codes:**
- `200 OK`: Package updated
- `404 Not Found`: Package does not exist
- `401 Unauthorized`: Not authorized to update package

---

#### DELETE /api/packages/{name}

Delete a package from the registry.

**Parameters:**
- `name` (path): Package name

**Status Codes:**
- `204 No Content`: Package deleted successfully
- `404 Not Found`: Package does not exist
- `401 Unauthorized`: Not authorized to delete package

---

### Versions

#### GET /api/packages/{name}/versions

List all versions of a package.

**Parameters:**
- `name` (path): Package name

**Response:**
```json
{
  "package": "std",
  "versions": [
    {
      "version": "0.1.0",
      "published_at": "2025-01-15T10:00:00Z"
    },
    {
      "version": "0.2.0",
      "published_at": "2025-01-20T15:30:00Z"
    }
  ]
}
```

**Status Codes:**
- `200 OK`: Successfully retrieved versions
- `404 Not Found`: Package does not exist

---

#### GET /api/packages/{name}/versions/{version}

Get details about a specific package version.

**Parameters:**
- `name` (path): Package name
- `version` (path): Version string (e.g., "1.2.3")

**Response:**
```json
{
  "package": "std",
  "version": "0.1.0",
  "description": "Standard library",
  "license": "MIT",
  "published_at": "2025-01-15T10:00:00Z",
  "dependencies": [],
  "download_url": "/api/packages/std/versions/0.1.0/download"
}
```

**Status Codes:**
- `200 OK`: Version found
- `404 Not Found`: Package or version does not exist

---

#### POST /api/packages/{name}/versions

Publish a new version of a package.

**Parameters:**
- `name` (path): Package name

**Request Body:**
```json
{
  "version": "0.2.0",
  "description": "New features added",
  "artifact": "<base64-encoded-package-data>"
}
```

**Status Codes:**
- `201 Created`: Version published successfully
- `409 Conflict`: Version already exists
- `400 Bad Request`: Invalid version or artifact
- `401 Unauthorized`: Not authorized to publish

---

#### GET /api/packages/{name}/versions/{version}/download

Download a package artifact.

**Parameters:**
- `name` (path): Package name
- `version` (path): Version string

**Response:**
Binary package artifact with appropriate Content-Type header.

**Status Codes:**
- `200 OK`: Download successful
- `404 Not Found`: Package or version does not exist

---

### Search

#### GET /api/search

Search for packages by name, description, or keywords.

**Query Parameters:**
- `q` (required): Search query
- `limit` (optional): Maximum number of results (default: 20)
- `offset` (optional): Pagination offset (default: 0)

**Example:**
```
GET /api/search?q=http&limit=10
```

**Response:**
```json
{
  "query": "http",
  "results": [
    {
      "id": "http",
      "description": "HTTP client and server implementation",
      "version": "0.1.0",
      "downloads": 1500
    }
  ],
  "total": 1,
  "limit": 10,
  "offset": 0
}
```

**Status Codes:**
- `200 OK`: Search completed successfully
- `400 Bad Request`: Invalid query parameters

---

#### GET /api/search/autocomplete

Get autocomplete suggestions for package names.

**Query Parameters:**
- `q` (required): Partial package name
- `limit` (optional): Maximum suggestions (default: 10)

**Example:**
```
GET /api/search/autocomplete?q=ht&limit=5
```

**Response:**
```json
{
  "suggestions": ["http", "html", "htmx"]
}
```

**Status Codes:**
- `200 OK`: Suggestions retrieved

---

### Users

#### POST /api/users/register

Register a new user account.

**Request Body:**
```json
{
  "username": "alice",
  "email": "alice@example.com",
  "password": "secure-password"
}
```

**Response:**
```json
{
  "username": "alice",
  "email": "alice@example.com",
  "created_at": "2025-01-15T10:00:00Z"
}
```

**Status Codes:**
- `201 Created`: User registered successfully
- `409 Conflict`: Username or email already exists
- `400 Bad Request`: Invalid registration data

---

#### POST /api/users/login

Authenticate and receive an access token.

**Request Body:**
```json
{
  "username": "alice",
  "password": "secure-password"
}
```

**Response:**
```json
{
  "token": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_at": "2025-01-16T10:00:00Z"
}
```

**Status Codes:**
- `200 OK`: Login successful
- `401 Unauthorized`: Invalid credentials

---

#### GET /api/users/{username}

Get user profile information.

**Parameters:**
- `username` (path): Username

**Response:**
```json
{
  "username": "alice",
  "email": "alice@example.com",
  "created_at": "2025-01-15T10:00:00Z",
  "packages": ["my-package", "another-package"]
}
```

**Status Codes:**
- `200 OK`: User found
- `404 Not Found`: User does not exist

---

### Statistics

#### GET /api/stats

Get overall registry statistics.

**Response:**
```json
{
  "total_packages": 150,
  "total_downloads": 50000,
  "total_users": 25,
  "recent_packages": 5
}
```

**Status Codes:**
- `200 OK`: Statistics retrieved

---

#### GET /api/stats/popular

Get most popular packages by download count.

**Query Parameters:**
- `limit` (optional): Number of packages to return (default: 10)

**Response:**
```json
{
  "packages": [
    {
      "id": "http",
      "downloads": 10000,
      "weekly_downloads": 500
    },
    {
      "id": "json",
      "downloads": 8500,
      "weekly_downloads": 450
    }
  ]
}
```

**Status Codes:**
- `200 OK`: Popular packages retrieved

---

## Status Codes

The API uses standard HTTP status codes:

| Code | Meaning |
|------|---------|
| 200  | OK - Request succeeded |
| 201  | Created - Resource created successfully |
| 204  | No Content - Request succeeded, no response body |
| 400  | Bad Request - Invalid request parameters or body |
| 401  | Unauthorized - Authentication required or failed |
| 403  | Forbidden - Authenticated but not authorized |
| 404  | Not Found - Resource does not exist |
| 409  | Conflict - Resource already exists |
| 500  | Internal Server Error - Server error occurred |

## Error Response Format

All error responses follow this format:

```json
{
  "error": "Human-readable error message"
}
```

## Authentication

Protected endpoints require an Authorization header:

```
Authorization: Bearer <token>
```

Obtain tokens via the `/api/users/login` endpoint.

## Rate Limiting

The API implements rate limiting:
- **Unauthenticated**: 60 requests per minute
- **Authenticated**: 300 requests per minute

Rate limit headers are included in all responses:
```
X-RateLimit-Limit: 300
X-RateLimit-Remaining: 275
X-RateLimit-Reset: 1642258800
```

## Versioning

The API version is included in all responses:
```
X-API-Version: 1.0.0
```

Future API versions will use URL versioning: `/api/v2/packages`
