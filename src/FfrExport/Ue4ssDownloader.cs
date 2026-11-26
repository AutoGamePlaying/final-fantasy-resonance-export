using System.Globalization;
using System.Net;
using System.Security.Cryptography;

namespace FfrExport;

internal interface IUe4ssPackageDownloader
{
    Task<string> EnsureVerifiedArchiveAsync(
        string downloadsDirectory,
        TextWriter output,
        CancellationToken cancellationToken);
}

/// <summary>
/// Downloads the pinned official UE4SS v4.0.0 release archive over HTTPS with strict size and SHA-256 verification.
/// This is the ONLY file in the repository permitted to use HttpClient; exporting and database building remain 100% offline.
/// </summary>
internal sealed class HttpsUe4ssPackageDownloader : IUe4ssPackageDownloader
{
    internal const string Ue4ssVersion = "v4.0.0";
    internal const string OfficialDownloadUrl =
        "https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip";
    internal const string OfficialArchiveSha256 =
        "251ad0d1381a241750e036f5918cbf2b5d7ec58ebc608cb2dd09d459d0c81e45";
    internal const long OfficialArchiveSize = 8_758_673L;
    internal const string OfficialArchiveFileName = "UE4SS_v4.0.0.zip";

    private const int MaxRedirects = 5;
    private const int BufferSize = 81920;
    private static readonly TimeSpan DefaultHttpTimeout = TimeSpan.FromSeconds(120);

    private readonly string _downloadUrl;
    private readonly long _expectedSize;
    private readonly string _expectedSha256;
    private readonly string _archiveFileName;
    private readonly TimeSpan _timeout;

    internal HttpsUe4ssPackageDownloader(
        string downloadUrl = OfficialDownloadUrl,
        long expectedSize = OfficialArchiveSize,
        string expectedSha256 = OfficialArchiveSha256,
        string archiveFileName = OfficialArchiveFileName,
        TimeSpan? timeout = null)
    {
        _downloadUrl = downloadUrl;
        _expectedSize = expectedSize;
        _expectedSha256 = expectedSha256.ToLowerInvariant();
        _archiveFileName = archiveFileName;
        _timeout = timeout ?? DefaultHttpTimeout;
    }

    public async Task<string> EnsureVerifiedArchiveAsync(
        string downloadsDirectory,
        TextWriter output,
        CancellationToken cancellationToken)
    {
        var fullDownloadsDir = PathSupport.TrimTrailingSeparators(Path.GetFullPath(downloadsDirectory));
        Directory.CreateDirectory(PathSupport.ToIoPath(fullDownloadsDir));

        var cachePath = Path.Combine(fullDownloadsDir, _archiveFileName);
        var ioCachePath = PathSupport.ToIoPath(cachePath);
        if (File.Exists(ioCachePath))
        {
            if (TryVerifyArchiveFile(cachePath, _expectedSize, _expectedSha256, out var mismatchReason))
            {
                output.WriteLine($"已命中本地下载缓存并通过 SHA-256 校验：{cachePath}");
                return cachePath;
            }

            output.WriteLine($"警告：本地缓存文件未通过校验（{mismatchReason}），将重新下载：{cachePath}");
            try
            {
                File.Delete(ioCachePath);
            }
            catch (Exception exception)
            {
                throw new IOException($"删除损坏的下载缓存失败（{cachePath}）：{exception.Message}", exception);
            }
        }

        if (!Uri.TryCreate(_downloadUrl, UriKind.Absolute, out var initialUri)
            || !string.Equals(initialUri.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase))
        {
            throw new InvalidOperationException($"拒绝使用非 HTTPS 地址下载 UE4SS：{_downloadUrl}");
        }

        output.WriteLine($"正在从官方 GitHub 发布页下载 UE4SS {Ue4ssVersion}（约 8.4 MB）：{_downloadUrl}");

        var tempPath = Path.Combine(
            fullDownloadsDir,
            $"{_archiveFileName}.{Environment.ProcessId}.{Guid.NewGuid():N}.tmp");
        try
        {
            await DownloadToFileAsync(initialUri, tempPath, cancellationToken).ConfigureAwait(false);
            VerifyArchiveFileOrThrow(tempPath, _expectedSize, _expectedSha256, _downloadUrl);
            File.Move(PathSupport.ToIoPath(tempPath), ioCachePath, overwrite: true);
            output.WriteLine($"下载并校验通过（大小 {_expectedSize} 字节，SHA-256 匹配）：{cachePath}");
            return cachePath;
        }
        catch
        {
            TryDeleteQuietly(tempPath);
            throw;
        }
    }

    internal static bool TryVerifyArchiveFile(
        string archivePath,
        long expectedSize,
        string expectedSha256Lower,
        out string? failureReason)
    {
        failureReason = null;
        var ioPath = PathSupport.ToIoPath(archivePath);
        if (!File.Exists(ioPath))
        {
            failureReason = $"文件不存在：{archivePath}";
            return false;
        }

        var actualSize = new FileInfo(ioPath).Length;
        if (actualSize != expectedSize)
        {
            failureReason = string.Create(
                CultureInfo.InvariantCulture,
                $"文件大小不匹配（期望 {expectedSize} 字节，实际 {actualSize} 字节）");
            return false;
        }

        var actualSha256 = GameVersionDetector.ComputeFileSha256Lower(archivePath);
        if (!string.Equals(actualSha256, expectedSha256Lower, StringComparison.OrdinalIgnoreCase))
        {
            failureReason = $"SHA-256 校验不一致（期望 {expectedSha256Lower}，实际 {actualSha256}）";
            return false;
        }

        return true;
    }

    internal static void VerifyArchiveFileOrThrow(
        string archivePath,
        long expectedSize,
        string expectedSha256Lower,
        string sourceDescription)
    {
        if (!TryVerifyArchiveFile(archivePath, expectedSize, expectedSha256Lower, out var reason))
        {
            throw new InvalidDataException(
                $"校验 UE4SS 安装包失败（来源：{sourceDescription}，文件：{archivePath}）：{reason}；已拒绝安装。");
        }
    }

    private async Task DownloadToFileAsync(Uri initialUri, string destinationPath, CancellationToken cancellationToken)
    {
        using var handler = new SocketsHttpHandler
        {
            AllowAutoRedirect = false,
            AutomaticDecompression = DecompressionMethods.None,
            ConnectTimeout = TimeSpan.FromSeconds(30),
        };
        using var client = new HttpClient(handler)
        {
            Timeout = _timeout,
        };
        client.DefaultRequestHeaders.UserAgent.ParseAdd("ffr-export/0.1");

        var currentUri = initialUri;
        HttpResponseMessage? response = null;

        try
        {
            for (var redirect = 0; redirect <= MaxRedirects; redirect++)
            {
                if (!string.Equals(currentUri.Scheme, Uri.UriSchemeHttps, StringComparison.OrdinalIgnoreCase))
                {
                    throw new InvalidOperationException($"下载重定向到非 HTTPS 地址，已拒绝：{currentUri}");
                }

                try
                {
                    response = await client
                        .GetAsync(currentUri, HttpCompletionOption.ResponseHeadersRead, cancellationToken)
                        .ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
                {
                    throw new TimeoutException(
                        string.Create(
                            CultureInfo.InvariantCulture,
                            $"下载 UE4SS 超时（超过 {(int)_timeout.TotalSeconds} 秒，地址：{currentUri}）；请检查网络或代理设置后重试。"));
                }
                catch (HttpRequestException exception)
                {
                    throw new IOException(
                        $"下载 UE4SS 失败（地址：{currentUri}）：{exception.Message}；请检查网络连接或防火墙设置后重试。",
                        exception);
                }

                if (IsRedirectStatusCode(response.StatusCode))
                {
                    var location = response.Headers.Location;
                    response.Dispose();
                    response = null;

                    if (location is null)
                    {
                        throw new IOException($"下载 UE4SS 失败：服务器返回重定向但缺少 Location 头（地址：{currentUri}）。");
                    }

                    currentUri = location.IsAbsoluteUri ? location : new Uri(currentUri, location);
                    if (redirect == MaxRedirects)
                    {
                        throw new IOException($"下载 UE4SS 失败：重定向次数超过 {MaxRedirects} 次（地址：{_downloadUrl}）。");
                    }

                    continue;
                }

                break;
            }

            if (response is null)
            {
                throw new IOException($"下载 UE4SS 失败：未收到服务器响应（地址：{_downloadUrl}）。");
            }

            if (!response.IsSuccessStatusCode)
            {
                throw new IOException(
                    string.Create(
                        CultureInfo.InvariantCulture,
                        $"下载 UE4SS 失败：服务器返回 HTTP {(int)response.StatusCode} {response.ReasonPhrase}（地址：{currentUri}）。"));
            }

            if (response.Content.Headers.ContentLength is long contentLength && contentLength != _expectedSize)
            {
                throw new InvalidDataException(
                    string.Create(
                        CultureInfo.InvariantCulture,
                        $"下载 UE4SS 被拒绝：响应头 Content-Length 为 {contentLength} 字节，与预期 {_expectedSize} 字节不一致（地址：{currentUri}）。"));
            }

            await using var remoteStream = await response.Content
                .ReadAsStreamAsync(cancellationToken)
                .ConfigureAwait(false);
            await using var localStream = new FileStream(
                PathSupport.ToIoPath(destinationPath),
                FileMode.CreateNew,
                FileAccess.Write,
                FileShare.None,
                BufferSize,
                useAsync: true);

            var buffer = new byte[BufferSize];
            long totalRead = 0;
            while (true)
            {
                int bytesRead;
                try
                {
                    bytesRead = await remoteStream
                        .ReadAsync(buffer.AsMemory(0, buffer.Length), cancellationToken)
                        .ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
                {
                    throw new TimeoutException(
                        string.Create(
                            CultureInfo.InvariantCulture,
                            $"下载 UE4SS 传输超时（已接收 {totalRead} / {_expectedSize} 字节，地址：{currentUri}）。"));
                }

                if (bytesRead == 0)
                {
                    break;
                }

                totalRead += bytesRead;
                if (totalRead > _expectedSize)
                {
                    throw new InvalidDataException(
                        string.Create(
                            CultureInfo.InvariantCulture,
                            $"下载 UE4SS 被拒绝：实际接收字节数超过预期 {_expectedSize} 字节（地址：{currentUri}）。"));
                }

                await localStream
                    .WriteAsync(buffer.AsMemory(0, bytesRead), cancellationToken)
                    .ConfigureAwait(false);
            }

            await localStream.FlushAsync(cancellationToken).ConfigureAwait(false);
        }
        finally
        {
            response?.Dispose();
        }
    }

    private static bool IsRedirectStatusCode(HttpStatusCode statusCode)
    {
        return statusCode is HttpStatusCode.MovedPermanently
            or HttpStatusCode.Found
            or HttpStatusCode.SeeOther
            or HttpStatusCode.TemporaryRedirect
            or HttpStatusCode.PermanentRedirect;
    }

    private static void TryDeleteQuietly(string path)
    {
        try
        {
            var ioPath = PathSupport.ToIoPath(path);
            if (File.Exists(ioPath))
            {
                File.Delete(ioPath);
            }
        }
        catch
        {
            // Ignore temp file cleanup failures.
        }
    }
}
